use std::fs;
use std::time::Duration;
use std::{collections::HashMap, sync::Arc};

use crate::statistics_model::Statistics;
use crate::DbPool;
use crate::{now_shanghai, SYSTEM_DOMAIN};

use crate::membership_model::Membership;
use anyhow::anyhow;
use axum::http::{HeaderMap, HeaderValue};
use chrono::{NaiveDateTime, NaiveTime};
use lazy_static::lazy_static;
use regex::Regex;
use serde::Serialize;
use serde_repr::*;
use tokio::sync::watch::{self, Receiver, Sender};
use tokio::sync::RwLock;
use tracing::info;

pub type DynContext = Arc<Context>;

lazy_static! {
    static ref IPV4_MASK: Regex = Regex::new("(\\d*\\.).*(\\.\\d*)").unwrap();
    static ref IPV6_MASK: Regex = Regex::new("(\\w*:\\w*:).*(:\\w*:\\w*)").unwrap();
}

#[derive(Serialize)]
struct VistEvent {
    ip: String,
    country: String,
    member: Membership,
    vt: Option<VisitorType>,
    referrer: Option<String>,
}

#[derive(Serialize_repr, Debug, PartialEq, Clone, Copy)]
#[repr(u8)]
pub enum VisitorType {
    Referer = 1,
    Badge = 2,
    ICON = 3,
    Favicon = 4,
    Card = 5,
}

pub struct Context {
    pub db_pool: DbPool,
    pub unique_visitor: RwLock<HashMap<i64, (i64, NaiveDateTime)>>,
    pub referrer: RwLock<HashMap<i64, (i64, NaiveDateTime)>>,
    pub rank_avg: RwLock<i64>,

    pub domain2id: HashMap<String, i64>,
    pub id2member: HashMap<i64, Membership>,

    pub visitor_tx: Sender<String>,
    pub visitor_rx: Receiver<String>,

    pub rank: RwLock<Vec<Statistics>>,
    pub monthly_rank: RwLock<Vec<Statistics>>,

    pub cache: r_cache::cache::Cache<String, ()>,
}

impl Context {
    pub async fn get_tend_from_uv_and_rv(&self, uv: i64, rv: i64) -> i64 {
        let tend = (uv + rv) / self.rank_avg.read().await.to_owned();
        if tend > 10 {
            return 10;
        } else if tend < 1 {
            return 1;
        }
        tend
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

    pub async fn boring_visitor(
        &self,
        v_type: Option<VisitorType>,
        query_member_domain: &str,
        headers: &HeaderMap,
    ) -> Result<(Membership, i64, i64, i64), anyhow::Error> {
        let mut member_domain = query_member_domain.to_string();
        let domain_referrer = Self::get_domain_from_referrer(&headers).unwrap_or("".to_string());
        if v_type.is_some_and(|v| v == VisitorType::Referer) {
            if domain_referrer.eq(&*SYSTEM_DOMAIN) {
                return Err(anyhow!("system domain"));
            }
            member_domain = domain_referrer.clone();
        }
        if let Some(id) = self.domain2id.get(&member_domain) {
            let default_header = HeaderValue::from_str("").unwrap();
            let ip = headers
                .get("CF-Connecting-IP")
                .unwrap_or(&default_header)
                .to_str()
                .unwrap();
            info!("ip {}", ip);

            let country = headers
                .get("CF-IPCountry")
                .unwrap_or(&default_header)
                .to_str()
                .unwrap();
            info!("country {}", country);

            let visitor_key = format!("{}_{}_{:?}", ip, id, v_type);
            let mut visitor_cache = self.cache.get(&visitor_key).await;

            if visitor_cache.is_none() {
                self.cache
                    .set(visitor_key, (), Some(Duration::from_secs(60 * 60 * 4)))
                    .await;
                if member_domain.ne(&domain_referrer) {
                    visitor_cache = Some(());
                }
            }

            let mut notification = false;

            let mut referrer = self.referrer.write().await;
            let mut dist_r = referrer
                .get(id)
                .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)))
                .to_owned();
            if v_type.is_some_and(|v| v == VisitorType::Referer) {
                if visitor_cache.is_none() {
                    dist_r.0 += 1;
                    dist_r.1 = now_shanghai();
                    referrer.insert(*id, dist_r);
                }
                notification = true;
            }
            drop(referrer);

            let mut uv = self.unique_visitor.write().await;
            let mut dist_uv = uv
                .get(id)
                .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)))
                .to_owned();
            if v_type.is_some_and(|v| v != VisitorType::Referer) {
                if visitor_cache.is_none() {
                    dist_uv.0 += 1;
                    dist_uv.1 = now_shanghai();
                    uv.insert(*id, dist_uv);
                }
                notification = true;
            }
            drop(uv);

            let tend = self.get_tend_from_uv_and_rv(dist_uv.0, dist_r.0).await;

            if notification {
                let mut member = self.id2member.get(id).unwrap().to_owned();
                member.description = "".to_string();
                member.github_username = "".to_string();
                let _ = self.visitor_tx.send(
                    serde_json::json!(VistEvent {
                        ip: IPV6_MASK
                            .replace_all(&IPV4_MASK.replace_all(&ip, "$1****$2"), "$1****$2")
                            .to_string(),
                        country: country.to_string(),
                        member,
                        referrer: match domain_referrer.as_str() {
                            "" => None,
                            _ => Some(domain_referrer),
                        },
                        vt: v_type,
                    })
                    .to_string(),
                );
            }

            return Ok((
                self.id2member.get(id).unwrap().clone(),
                dist_uv.0,
                dist_r.0,
                tend,
            ));
        }
        Err(anyhow!("not a member"))
    }

    pub async fn default(db_pool: DbPool) -> Context {
        let statistics = Statistics::today(db_pool.get().unwrap()).unwrap_or_default();

        let mut page_view: HashMap<i64, (i64, NaiveDateTime)> = HashMap::new();
        let mut referrer: HashMap<i64, (i64, NaiveDateTime)> = HashMap::new();

        statistics.iter().for_each(|s| {
            page_view.insert(s.membership_id, (s.unique_visitor, s.updated_at));
            referrer.insert(
                s.membership_id,
                (
                    s.referrer,
                    s.latest_referrer_at
                        .unwrap_or(NaiveDateTime::from_timestamp(0, 0)),
                ),
            );
        });

        let mut membership: HashMap<i64, Membership> =
            serde_json::from_str(&fs::read_to_string("./resources/membership.json").unwrap())
                .unwrap();
        membership.retain(|_, v| v.hidden.is_none() || !v.hidden.unwrap());

        let mut domain2id: HashMap<String, i64> = HashMap::new();
        membership.iter_mut().for_each(|(k, v)| {
            v.id = *k; // 将 ID 补给 member
            domain2id.insert(v.domain.clone(), *k);
        });

        let rank = Statistics::rank_between(
            db_pool.get().unwrap(),
            NaiveDateTime::from_timestamp(0, 0),
            now_shanghai(),
        )
        .unwrap();

        let monthly_rank = Statistics::rank_between(
            db_pool.get().unwrap(),
            now_shanghai() - chrono::Duration::days(30),
            now_shanghai(),
        )
        .unwrap();

        let (visitor_tx, visitor_rx) = watch::channel::<String>("".to_string());

        let rank_svg = Statistics::prev_day_rank_avg(db_pool.get().unwrap());

        Context {
            db_pool,

            unique_visitor: RwLock::new(page_view),
            referrer: RwLock::new(referrer),
            rank_avg: RwLock::new(rank_svg),
            rank: RwLock::new(rank),
            monthly_rank: RwLock::new(monthly_rank),

            domain2id,
            id2member: membership,

            visitor_rx,
            visitor_tx,

            cache: r_cache::cache::Cache::new(Some(Duration::from_secs(60 * 10))),
        }
    }

    // 每五分钟存一次，发现隔天刷新
    pub async fn save_per_5_minutes(&self) {
        let mut uv_cache: HashMap<i64, (i64, NaiveDateTime)> = HashMap::new();
        let mut referrer_cache: HashMap<i64, (i64, NaiveDateTime)> = HashMap::new();
        let mut changed_list: Vec<i64> = Vec::new();
        let mut _today = NaiveDateTime::new(now_shanghai().date(), NaiveTime::from_hms(0, 0, 0));
        let id_list = Vec::from_iter(self.id2member.keys());
        loop {
            tokio::time::sleep(Duration::from_secs(60 * 5)).await;
            changed_list.clear();
            // 对比是否有数据更新
            let mut uv_write = self.unique_visitor.write().await;
            let mut referrer_write = self.referrer.write().await;
            id_list.iter().for_each(|id| {
                let uv = *uv_cache
                    .get(id)
                    .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)));
                let new_uv = *uv_write
                    .get(id)
                    .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)));
                if uv.0.ne(&new_uv.0) {
                    uv_cache.insert(**id, new_uv);
                    changed_list.push(**id);
                }
                let referrer = *referrer_cache
                    .get(id)
                    .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)));
                let new_referrer = *referrer_write
                    .get(id)
                    .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)));
                if referrer.0.ne(&new_referrer.0) {
                    referrer_cache.insert(**id, new_referrer);
                    if !changed_list.contains(id) {
                        changed_list.push(**id);
                    }
                }
            });
            // 更新到数据库
            changed_list.iter().for_each(|id| {
                let id_uv = *uv_cache
                    .get(id)
                    .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)));
                let id_referrer = *referrer_cache
                    .get(id)
                    .unwrap_or(&(0, NaiveDateTime::from_timestamp(0, 0)));
                Statistics::insert_or_update(
                    self.db_pool.get().unwrap(),
                    &Statistics {
                        created_at: _today,
                        membership_id: *id,
                        unique_visitor: id_uv.0,
                        updated_at: id_uv.1,
                        referrer: id_referrer.0,
                        latest_referrer_at: Some(id_referrer.1),
                        id: 0,
                    },
                )
                .unwrap();
            });
            let new_day = NaiveDateTime::new(now_shanghai().date(), NaiveTime::from_hms(0, 0, 0));
            if new_day.ne(&_today) {
                _today = new_day;
                // 如果是跨天重置数据
                uv_write.clear();
                referrer_write.clear();
                uv_cache.clear();
                referrer_cache.clear();
                // 重置访问打点
                self.cache.clear().await;
                // 更新上日访问量均值
                let mut rank_avg = self.rank_avg.write().await;
                *rank_avg = Statistics::prev_day_rank_avg(self.db_pool.get().unwrap());
            }
            drop(uv_write);
            drop(referrer_write);

            let mut rank = self.rank.write().await;
            *rank = Statistics::rank_between(
                self.db_pool.get().unwrap(),
                NaiveDateTime::from_timestamp(0, 0),
                now_shanghai(),
            )
            .unwrap();

            let mut monthly_rank = self.monthly_rank.write().await;
            *monthly_rank = Statistics::rank_between(
                self.db_pool.get().unwrap(),
                now_shanghai() - chrono::Duration::days(30),
                now_shanghai(),
            )
            .unwrap();
        }
    }
}
