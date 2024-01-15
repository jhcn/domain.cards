use std::{collections::HashMap, io::Read, sync::Arc, time::Duration};

use askama::Template;
use axum::{
    extract::{
        ws::{Message, WebSocket},
        Extension, Path, Query, WebSocketUpgrade,
    },
    http::{header::HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::NaiveDateTime;
use lazy_static::lazy_static;
use rand::seq::SliceRandom;
use rand::thread_rng;
use tokio::select;

use crate::{
    app_model::{Context, DynContext, VisitorType},
    membership_model::{Membership, RankAndMembership},
    now_shanghai,
    statistics_model::Statistics,
    GIT_HASH,
};

lazy_static! {
    static ref BADGE_CONTENT: String = {
        let mut s = String::new();
        std::fs::File::open("templates/assets/img/badge.svg")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
    static ref CARD_CONTENT: String = {
        let mut s = String::new();
        std::fs::File::open("templates/assets/img/card.svg")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
    static ref ICON_CONTENT: String = {
        let mut s = String::new();
        std::fs::File::open("templates/assets/img/logo.svg")
            .unwrap()
            .read_to_string(&mut s)
            .unwrap();
        s
    };
}

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
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> Response {
    let tend = ctx
        .boring_visitor(Some(VisitorType::Badge), &domain, &headers)
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            ([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        )
            .into_response();
    }

    let tend = tend.unwrap();

    (
        StatusCode::OK,
        [("content-type", "image/svg+xml")],
        BADGE_CONTENT
            .replace("domain.fans", &tend.0.domain)
            .replace("1233", &tend.1.to_string())
            .replace("1244", &tend.2.to_string())
            .replace("1255", &tend.3.to_string()),
    )
        .into_response()
}

pub async fn show_card(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> Response {
    let tend = ctx
        .boring_visitor(Some(VisitorType::Card), &domain, &headers)
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            ([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        )
            .into_response();
    }

    let tend = tend.unwrap();

    let avatar_img_base64 = match std::fs::read(format!("resources/avatar/{}.png", &tend.0.id)) {
        Ok(img) => STANDARD.encode(img),
        Err(_) => {
            "iVBORw0KGgoAAAANSUhEUgAAAAgAAAAIAQMAAAD+wSzIAAAABlBMVEX///+/v7+jQ3Y5AAAADklEQVQI12P4AIX8EAgALgAD/aNpbtEAAAAASUVORK5CYII".to_string()
        }
    };

    (
        StatusCode::OK,
        [("content-type", "image/svg+xml")],
        CARD_CONTENT
            .replace("iVBORw0KGgoAAAANSUhEUgAAAAgAAAAIAQMAAAD+wSzIAAAABlBMVEX///+/v7+jQ3Y5AAAADklEQVQI12P4AIX8EAgALgAD/aNpbtEAAAAASUVORK5CYII", &avatar_img_base64)
            .replace("熊宝的米表", &tend.0.name)
            .replace("资深域名玩家，擅长新顶、单字符等。", &tend.0.description)
            .replace("domain.fans", &tend.0.domain)
            .replace("1233", &tend.1.to_string())
            .replace("1244", &tend.2.to_string())
            .replace("1255", &tend.3.to_string()),
    )
        .into_response()
}

pub async fn show_favicon(
    Path(domain): Path<String>,
    headers: HeaderMap,
    Extension(ctx): Extension<DynContext>,
) -> Response {
    let tend = ctx
        .boring_visitor(
            Some(crate::app_model::VisitorType::Favicon),
            &domain,
            &headers,
        )
        .await;
    if tend.is_err() {
        return (
            StatusCode::NOT_FOUND,
            ([("content-type", "text/plain")]),
            tend.err().unwrap().to_string(),
        )
            .into_response();
    }
    (
        StatusCode::OK,
        [("content-type", "image/svg+xml")],
        ICON_CONTENT.to_string(),
    )
        .into_response()
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
        return (StatusCode::NOT_FOUND, tend.err().unwrap().to_string()).into_response();
    }
    (
        StatusCode::OK,
        [("content-type", "image/svg+xml")],
        ICON_CONTENT.to_string(),
    )
        .into_response()
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
    let _ = ctx
        .boring_visitor(Some(crate::app_model::VisitorType::Referer), "", &headers)
        .await;

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
        if uv.0 > 0 || rv.0 > 0 || rank_type == "random" {
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
                        unique_visitor: match uv_read.get(&v.0) {
                            Some(uv) => uv.0,
                            None => 0,
                        },
                        referrer: match referrer_read.get(&v.0) {
                            Some(rv) => rv.0,
                            None => 0,
                        },
                        latest_referrer_at: Some(match referrer_read.get(&v.0) {
                            Some(rv) => rv.1,
                            None => NaiveDateTime::from_timestamp(0, 0),
                        }),
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
    let _ = ctx
        .boring_visitor(Some(crate::app_model::VisitorType::Referer), "", &headers)
        .await;

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
