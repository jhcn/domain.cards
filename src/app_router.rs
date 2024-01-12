use std::{collections::HashMap, sync::Arc, time::Duration};

use anyhow::anyhow;
use askama::Template;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, Path, Query, WebSocketUpgrade,
    },
    http::{header::HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use chrono::NaiveDateTime;
use rand::seq::SliceRandom;
use rand::thread_rng;
use tokio::select;

use crate::{
    app_model::{Context, DynContext},
    boring_face::BoringFace,
    membership_model::{Membership, RankAndMembership},
    now_shanghai,
    statistics_model::Statistics,
    GIT_HASH,
};

pub async fn ws_upgrade(
    Extension(ctx): Extension<DynContext>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(|socket| handle_socket(ctx, socket))
}

async fn handle_socket(ctx: Arc<Context>, mut socket: WebSocket) {
    let mut rx = ctx.visitor_rx.clone();
    let mut interval = tokio::time::interval(Duration::from_secs(8));

    loop {
        select! {
            Ok(()) = rx.changed() => {
                let msg = rx.borrow().to_string();
                let res = socket.send(Message::Text(msg.clone())).await;
                if res.is_err() {
                    break;
                }
            }
            _ = interval.tick() => {
                let res = socket.send(Message::Ping(vec![])).await;
                if res.is_err() {
                    break;
                }
            }
        }
    }
}

pub async fn show_badge(
    Path(mut domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> Response {
    let mut v_type = Some(crate::app_model::VisitorType::Badge);

    let domain_referrer = get_domain_from_referrer(&headers).unwrap_or("".to_string());
    if domain_referrer.ne(&domain) {
        if domain.eq("[domain]") {
            domain = domain_referrer;
        } else {
            v_type = None;
        }
    }

    let tend = ctx.boring_visitor(v_type, &domain, &headers).await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            ([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        )
            .into_response();
    }

    render_svg(tend.unwrap(), &ctx.badge).await
}

pub async fn show_favicon(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> Response {
    let tend = ctx
        .boring_visitor(Some(crate::app_model::VisitorType::ICON), &domain, &headers)
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            ([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        )
            .into_response();
    }
    render_svg(tend.unwrap(), &ctx.favicon).await
}

pub async fn show_icon(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> Response {
    let tend = ctx
        .boring_visitor(Some(crate::app_model::VisitorType::ICON), &domain, &headers)
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            ([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        )
            .into_response();
    }

    render_svg(tend.unwrap(), &ctx.icon).await
}

#[derive(Template)]
#[template(path = "index.html")]
struct HomeTemplate {
    version: String,
    rank: Vec<RankAndMembership>,
    rank_type: String,
    level: HashMap<i64, i64>,
}

pub async fn home_page(
    Extension(ctx): Extension<DynContext>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Html<String>, String> {
    let domain = get_domain_from_referrer(&headers);
    if domain.is_ok() {
        let _ = ctx
            .boring_visitor(
                Some(crate::app_model::VisitorType::Referer),
                &domain.unwrap(),
                &headers,
            )
            .await;
    }

    let mut rank_type = query
        .get("rank_type")
        .unwrap_or(&"daily".to_string())
        .clone();
    if !["daily", "monthly", "random"].contains(&rank_type.as_str()) {
        rank_type = "daily".to_string();
    }

    let referrer_read = ctx.referrer.read().await;
    let uv_read = ctx.unique_visitor.read().await;

    let mut level: HashMap<i64, i64> = HashMap::new();
    let mut rank_vec: Vec<(i64, NaiveDateTime, i64)> = Vec::new();

    for k in ctx.id2member.keys() {
        let uv = uv_read
            .get(k)
            .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)))
            .to_owned();
        let rv = referrer_read
            .get(k)
            .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)))
            .to_owned();
        if uv.0 > 0 || rv.0 > 0 {
            rank_vec.push((k.to_owned(), rv.1, uv.0));
            level.insert(k.to_owned(), ctx.get_tend_from_uv_and_rv(uv.0, rv.0).await);
        }
    }

    let rank = match rank_type.as_str() {
        "daily" => {
            rank_vec.sort_by(|a, b| match b.1.cmp(&a.1) {
                std::cmp::Ordering::Equal => b.2.cmp(&a.2),
                _ => b.1.cmp(&a.1),
            });
            let mut rank_daily = Vec::new();
            for v in rank_vec {
                if rank_daily.len() >= 30 {
                    break;
                }
                rank_daily.push(RankAndMembership {
                    rank: Statistics {
                        id: 0,
                        created_at: now_shanghai(),
                        updated_at: now_shanghai(),
                        membership_id: v.0,
                        unique_visitor: uv_read.get(&v.0).unwrap().0,
                        referrer: referrer_read.get(&v.0).unwrap().0,
                        latest_referrer_at: Some(referrer_read.get(&v.0).unwrap().1),
                    },
                    membership: ctx.id2member.get(&v.0).unwrap().to_owned(),
                });
            }
            rank_daily
        }
        "monthly" => {
            let mut rank_monthly = Vec::new();
            let monthly_rank = ctx.monthly_rank.read().await.to_owned();
            monthly_rank
                .iter()
                .filter(|r| ctx.id2member.contains_key(&r.membership_id))
                .for_each(|r| {
                    if rank_monthly.len() >= 30
                        || r.updated_at < now_shanghai() - chrono::Duration::days(30)
                    {
                        return;
                    }
                    let m = ctx.id2member.get(&r.membership_id).unwrap().to_owned();
                    rank_monthly.push(RankAndMembership {
                        rank: r.to_owned(),
                        membership: m,
                    });
                });
            rank_monthly
        }
        "random" => {
            let mut rank_and_membership_to_be_remove = Vec::new();
            let mut rank_and_membership = Vec::new();
            let rank = ctx.rank.read().await.to_owned();
            rank.iter()
                .filter(|r| ctx.id2member.contains_key(&r.membership_id))
                .for_each(|r| {
                    if r.updated_at > now_shanghai() - chrono::Duration::days(30) {
                        let m = ctx.id2member.get(&r.membership_id).unwrap().to_owned();
                        rank_and_membership.push(RankAndMembership {
                            rank: r.to_owned(),
                            membership: m,
                        });
                    } else {
                        let m: Membership = ctx.id2member.get(&r.membership_id).unwrap().to_owned();
                        rank_and_membership_to_be_remove.push(RankAndMembership {
                            rank: r.to_owned(),
                            membership: m,
                        });
                    }
                });
            rank_and_membership.shuffle(&mut thread_rng());
            rank_and_membership.into_iter().take(30).collect()
        }
        _ => {
            vec![]
        }
    };

    let tpl = HomeTemplate {
        rank,
        rank_type,
        level,
        version: GIT_HASH[0..8].to_string(),
    };
    let html = tpl.render().map_err(|err| err.to_string())?;
    Ok(Html(html))
}

#[derive(Template)]
#[template(path = "join_us.html")]
struct JoinUsTemplate {
    version: String,
}

pub async fn join_us_page() -> Result<Html<String>, String> {
    let tpl = JoinUsTemplate {
        version: GIT_HASH[0..8].to_string(),
    };
    let html = tpl.render().map_err(|err| err.to_string())?;
    Ok(Html(html))
}

#[derive(Template)]
#[template(path = "rank.html")]
struct RankTemplate {
    version: String,
    rank: Vec<RankAndMembership>,
    to_be_remove: Vec<RankAndMembership>,
}

pub async fn rank_page(
    Extension(ctx): Extension<DynContext>,
    headers: HeaderMap,
) -> Result<Html<String>, String> {
    let domain = get_domain_from_referrer(&headers);
    if domain.is_ok() {
        let _ = ctx
            .boring_visitor(
                Some(crate::app_model::VisitorType::Referer),
                &domain.unwrap(),
                &headers,
            )
            .await;
    }

    let rank = ctx.rank.read().await.to_owned();

    let mut rank_and_membership_to_be_remove = Vec::new();

    let mut rank_and_membership = Vec::new();

    rank.iter()
        .filter(|r| ctx.id2member.contains_key(&r.membership_id))
        .for_each(|r| {
            if r.updated_at > now_shanghai() - chrono::Duration::days(30) {
                let m = ctx.id2member.get(&r.membership_id).unwrap().to_owned();
                rank_and_membership.push(RankAndMembership {
                    rank: r.to_owned(),
                    membership: m,
                });
            } else {
                let m = ctx.id2member.get(&r.membership_id).unwrap().to_owned();
                rank_and_membership_to_be_remove.push(RankAndMembership {
                    rank: r.to_owned(),
                    membership: m,
                });
            }
        });

    let tpl = RankTemplate {
        rank: rank_and_membership,
        to_be_remove: rank_and_membership_to_be_remove,
        version: GIT_HASH[0..8].to_string(),
    };
    let html = tpl.render().map_err(|err| err.to_string())?;
    Ok(Html(html))
}

fn get_domain_from_referrer(headers: &HeaderMap) -> Result<String, anyhow::Error> {
    let referrer_header = headers.get("Referer");
    if referrer_header.is_none() {
        return Err(anyhow!("no referrer header"));
    }

    let referrer_str = String::from_utf8(referrer_header.unwrap().as_bytes().to_vec());
    if referrer_str.is_err() {
        return Err(anyhow!("referrer header is not valid utf-8 string"));
    }

    let referrer_url = url::Url::parse(&referrer_str.unwrap());
    if referrer_url.is_err() {
        return Err(anyhow!("referrer header is not valid URL"));
    }

    let referrer_url = referrer_url.unwrap();
    if referrer_url.domain().is_none() {
        return Err(anyhow!("referrer header doesn't contains a valid domain"));
    }

    return Ok(referrer_url.domain().unwrap().to_string());
}

async fn render_svg(tend: (&str, i64, i64, i64), render: &BoringFace) -> Response {
    (
        StatusCode::OK,
        ([("content-type", "image/svg+xml")]),
        render.render_svg(tend.0, tend.1, tend.2, tend.3),
    )
        .into_response()
}
