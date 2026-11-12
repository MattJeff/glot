//! Where "what locals search" comes from. One trait, and only sources that are free **and** allowed by their terms:
//!
//! | source | status | why |
//! |---|---|---|
//! | [`Manual`]: the customer's own keyword list | built | their data, their tools |
//! | [`SearchConsole`]: Google Search Console API, the customer's own OAuth consent, read-only | built | official, free, per-country queries of the customer's own site |
//! | `MockDemand` | tests only | never feeds the product |
//! | Bing Webmaster API (`GetQueryStats`, customer's API key) | adapter to write | official and free; small share of most markets |
//! | Google Trends API | not available | official API is an application-only alpha since July 2025 |
//! | Google Autocomplete (`suggestqueries`) | refused | unpublished endpoint; automated queries are against Google's terms |
//! | DataForSEO, Semrush, Ahrefs, Keywords Everywhere, SerpApi, Google Ads Keyword Planner | paid / restricted | adapters to write behind this trait, each with the customer's own key; never called here |
//!
//! Search Console only knows queries the site already appears for. In a market the site has never ranked in it
//! returns nothing, and the market then says "no search data yet" (`needs_demand`): keywords and volumes are never
//! estimated or invented.
use async_trait::async_trait;
use chrono::NaiveDate;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
#[cfg(test)]
use std::collections::BTreeMap;
use std::time::Duration;
use uuid::Uuid;

use super::validate::Market;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Term {
    pub term: String,
    /// manual | gsc (this page) | gsc_site (the whole site in that country)
    pub source: String,
    /// `None` = unknown. Never invented.
    pub impressions: Option<i64>,
    pub clicks: Option<i64>,
    pub position: Option<f64>,
}

pub struct Query<'a> {
    pub market: &'a Market,
    pub path: &'a str,
    /// The page's URL in this market, when the site's URL scheme is known.
    pub page_url: Option<&'a str>,
}

/// A source of demand terms. An `Err` is a sentence for the dashboard: the scan goes on with the other sources.
#[async_trait]
pub trait DemandSource: Send + Sync {
    fn id(&self) -> &'static str;
    async fn terms(&self, q: &Query<'_>) -> Result<Vec<Term>, String>;
}

/// Best first: known volume before unknown, then alphabetical (stable across runs). One entry per term.
pub fn rank(mut terms: Vec<Term>) -> Vec<Term> {
    terms.sort_by(|a, b| {
        b.impressions
            .unwrap_or(-1)
            .cmp(&a.impressions.unwrap_or(-1))
            .then_with(|| a.term.cmp(&b.term))
    });
    let mut seen = std::collections::HashSet::new();
    terms.retain(|t| seen.insert(t.term.to_lowercase()));
    terms.truncate(20);
    terms
}

// ---------- mock (tests only) ----------

/// Fixed terms per locale, for tests of code written against the trait. Never wired into the product: a market
/// without real demand data shows "no search data yet", not sample keywords.
#[cfg(test)]
pub struct MockDemand(pub BTreeMap<String, Vec<(String, i64)>>);

#[cfg(test)]
#[async_trait]
impl DemandSource for MockDemand {
    fn id(&self) -> &'static str {
        "mock"
    }
    async fn terms(&self, q: &Query<'_>) -> Result<Vec<Term>, String> {
        Ok(self
            .0
            .get(&q.market.locale)
            .into_iter()
            .flatten()
            .map(|(term, n)| Term {
                term: term.clone(),
                source: "mock".into(),
                impressions: Some(*n),
                clicks: None,
                position: None,
            })
            .collect())
    }
}

// ---------- manual ----------

/// The customer's own keyword list (`PUT …/local-seo/demand`): terms of the page, then of the whole market.
pub struct Manual {
    pub db: sqlx::PgPool,
    pub project: Uuid,
}

#[async_trait]
impl DemandSource for Manual {
    fn id(&self) -> &'static str {
        "manual"
    }
    async fn terms(&self, q: &Query<'_>) -> Result<Vec<Term>, String> {
        let rows: Vec<(String, Option<i64>)> = sqlx::query_as(
            "SELECT term, impressions FROM local_seo_demand
             WHERE project_id = $1 AND locale = $2 AND source = 'manual' AND path IN ($3, '')
             ORDER BY (path = '')",
        )
        .bind(self.project)
        .bind(&q.market.locale)
        .bind(q.path)
        .fetch_all(&self.db)
        .await
        .map_err(|e| format!("could not read the keyword list: {e}"))?;
        Ok(rows
            .into_iter()
            .map(|(term, impressions)| Term {
                term,
                source: "manual".into(),
                impressions,
                clicks: None,
                position: None,
            })
            .collect())
    }
}

// ---------- Google Search Console ----------

pub const GSC_SCOPE: &str = "https://www.googleapis.com/auth/webmasters.readonly";

/// The founder's Google OAuth client (`GOOGLE_OAUTH_CLIENT_ID` / `GOOGLE_OAUTH_CLIENT_SECRET`). Endpoints are fields
/// so tests point them at a local fake.
#[derive(Debug, Clone)]
pub struct Google {
    pub client_id: String,
    pub client_secret: String,
    pub auth_url: String,
    pub token_url: String,
    pub api_base: String,
}

impl Google {
    pub fn from_env() -> Option<Self> {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        Some(Self {
            client_id: var("GOOGLE_OAUTH_CLIENT_ID")?,
            client_secret: var("GOOGLE_OAUTH_CLIENT_SECRET")?,
            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token_url: "https://oauth2.googleapis.com/token".into(),
            api_base: "https://www.googleapis.com".into(),
        })
    }

    fn http() -> reqwest::Client {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .expect("reqwest client")
    }

    /// Where the customer consents (read-only Search Console scope, offline access for the refresh token).
    pub fn consent_url(&self, redirect_uri: &str, state: &str) -> String {
        Url::parse_with_params(
            &self.auth_url,
            [
                ("client_id", self.client_id.as_str()),
                ("redirect_uri", redirect_uri),
                ("response_type", "code"),
                ("scope", GSC_SCOPE),
                ("access_type", "offline"),
                ("prompt", "consent"),
                ("state", state),
            ],
        )
        .map(String::from)
        .unwrap_or_default()
    }

    async fn token(&self, params: &[(&str, &str)]) -> Result<Value, String> {
        let mut form = vec![
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
        ];
        form.extend_from_slice(params);
        let res = Self::http()
            .post(&self.token_url)
            .form(&form)
            .send()
            .await
            .map_err(|_| "Google did not answer; try again in a minute.".to_string())?;
        let ok = res.status().is_success();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        if !ok {
            return Err(match body["error"].as_str() {
                Some("invalid_grant") => "Google no longer accepts this connection (access was revoked or expired): connect Search Console again.".into(),
                Some(e) => format!("Google refused the connection ({e})."),
                None => "Google refused the connection.".into(),
            });
        }
        Ok(body)
    }

    /// Authorization code → refresh token.
    pub async fn exchange(&self, code: &str, redirect_uri: &str) -> Result<String, String> {
        let body = self
            .token(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", redirect_uri),
            ])
            .await?;
        body["refresh_token"].as_str().map(str::to_string).ok_or_else(|| {
            "Google returned no refresh token: remove glot from your Google account's third-party access, then connect again.".into()
        })
    }

    pub async fn access_token(&self, refresh_token: &str) -> Result<String, String> {
        let body = self
            .token(&[
                ("grant_type", "refresh_token"),
                ("refresh_token", refresh_token),
            ])
            .await?;
        body["access_token"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "Google returned no access token.".into())
    }

    /// Search Console properties the account can read.
    pub async fn sites(&self, access_token: &str) -> Result<Vec<String>, String> {
        let res = Self::http()
            .get(format!("{}/webmasters/v3/sites", self.api_base))
            .bearer_auth(access_token)
            .send()
            .await
            .map_err(|_| "Search Console did not answer.".to_string())?;
        if !res.status().is_success() {
            return Err(format!(
                "Search Console refused to list your sites (HTTP {}).",
                res.status().as_u16()
            ));
        }
        let body: Value = res.json().await.unwrap_or(Value::Null);
        Ok(body["siteEntry"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|s| s["permissionLevel"] != "siteUnverifiedUser")
            .filter_map(|s| s["siteUrl"].as_str().map(str::to_string))
            .collect())
    }
}

/// The property that covers `site` (`https://example.com`): the domain property first, then the longest URL prefix.
pub fn pick_property(properties: &[String], site: &str) -> Option<String> {
    let url = Url::parse(site).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let domain = properties
        .iter()
        .filter(|p| {
            p.strip_prefix("sc-domain:")
                .is_some_and(|d| host == d || host.ends_with(&format!(".{d}")))
        })
        .max_by_key(|p| p.len());
    let prefix = properties
        .iter()
        .filter(|p| {
            !p.starts_with("sc-domain:")
                && format!("{}/", site.trim_end_matches('/')).starts_with(p.as_str())
        })
        .max_by_key(|p| p.len());
    domain.or(prefix).cloned()
}

/// ISO 3166-1 alpha-2 → the lowercase alpha-3 code Search Console filters by.
pub fn alpha3(country: &str) -> Option<&'static str> {
    const T: &[(&str, &str)] = &[
        ("AE", "are"),
        ("AR", "arg"),
        ("AT", "aut"),
        ("AU", "aus"),
        ("BE", "bel"),
        ("BG", "bgr"),
        ("BO", "bol"),
        ("BR", "bra"),
        ("CA", "can"),
        ("CH", "che"),
        ("CL", "chl"),
        ("CN", "chn"),
        ("CO", "col"),
        ("CR", "cri"),
        ("CZ", "cze"),
        ("DE", "deu"),
        ("DK", "dnk"),
        ("DO", "dom"),
        ("DZ", "dza"),
        ("EC", "ecu"),
        ("EE", "est"),
        ("EG", "egy"),
        ("ES", "esp"),
        ("FI", "fin"),
        ("FR", "fra"),
        ("GB", "gbr"),
        ("GR", "grc"),
        ("GT", "gtm"),
        ("HK", "hkg"),
        ("HR", "hrv"),
        ("HU", "hun"),
        ("ID", "idn"),
        ("IE", "irl"),
        ("IL", "isr"),
        ("IN", "ind"),
        ("IT", "ita"),
        ("JP", "jpn"),
        ("KR", "kor"),
        ("LT", "ltu"),
        ("LU", "lux"),
        ("LV", "lva"),
        ("MA", "mar"),
        ("MX", "mex"),
        ("MY", "mys"),
        ("NG", "nga"),
        ("NL", "nld"),
        ("NO", "nor"),
        ("NZ", "nzl"),
        ("PA", "pan"),
        ("PE", "per"),
        ("PH", "phl"),
        ("PL", "pol"),
        ("PT", "prt"),
        ("PY", "pry"),
        ("RO", "rou"),
        ("RS", "srb"),
        ("RU", "rus"),
        ("SA", "sau"),
        ("SE", "swe"),
        ("SG", "sgp"),
        ("SI", "svn"),
        ("SK", "svk"),
        ("TH", "tha"),
        ("TN", "tun"),
        ("TR", "tur"),
        ("TW", "twn"),
        ("UA", "ukr"),
        ("US", "usa"),
        ("UY", "ury"),
        ("VE", "ven"),
        ("VN", "vnm"),
        ("ZA", "zaf"),
    ];
    T.iter()
        .find(|(a2, _)| a2.eq_ignore_ascii_case(country))
        .map(|(_, a3)| *a3)
}

/// One customer's Search Console property.
pub struct SearchConsole {
    pub google: Google,
    pub site_url: String,
    pub refresh_token: String,
}

/// Search Console lags two to three days; the last 28 complete days are what its own UI shows by default.
const LAG_DAYS: i64 = 3;

impl SearchConsole {
    async fn query(&self, access: &str, body: Value) -> Result<Vec<Value>, String> {
        let mut url = Url::parse(&self.google.api_base).map_err(|e| e.to_string())?;
        url.path_segments_mut()
            .map_err(|_| "invalid Search Console URL".to_string())?
            .extend([
                "webmasters",
                "v3",
                "sites",
                self.site_url.as_str(),
                "searchAnalytics",
                "query",
            ]);
        let res = Google::http()
            .post(url)
            .bearer_auth(access)
            .json(&body)
            .send()
            .await
            .map_err(|_| "Search Console did not answer; try again in a minute.".to_string())?;
        let status = res.status().as_u16();
        let body: Value = res.json().await.unwrap_or(Value::Null);
        match status {
            200..=299 => Ok(body["rows"].as_array().cloned().unwrap_or_default()),
            403 => Err(format!("The Google account that connected Search Console cannot read {} any more: connect again with an account that owns the property.", self.site_url)),
            429 => Err("Search Console's quota is used up for now; the scan will work again in a few minutes.".into()),
            s => Err(format!("Search Console answered HTTP {s}.")),
        }
    }

    fn filters(market: &Market, page_url: Option<&str>) -> Vec<Value> {
        let mut f = vec![];
        if let Some(c) = market.country.as_deref().and_then(alpha3) {
            f.push(json!({"dimension": "country", "operator": "equals", "expression": c}));
        }
        if let Some(u) = page_url {
            f.push(json!({"dimension": "page", "operator": "equals", "expression": u}));
        }
        f
    }

    fn window(days: i64) -> (String, String) {
        let end = chrono::Utc::now().date_naive() - chrono::Duration::days(LAG_DAYS);
        (
            (end - chrono::Duration::days(days - 1)).to_string(),
            end.to_string(),
        )
    }

    /// Impressions, clicks and average position per day of one page in one market, last `days` days.
    pub async fn daily(
        &self,
        market: &Market,
        page_url: &str,
        days: i64,
    ) -> Result<Vec<(NaiveDate, i64, i64, f64)>, String> {
        let access = self.google.access_token(&self.refresh_token).await?;
        let (start, end) = Self::window(days);
        let rows = self
            .query(
                &access,
                json!({
                    "startDate": start, "endDate": end, "dimensions": ["date"], "rowLimit": 1000,
                    "dimensionFilterGroups": [{"filters": Self::filters(market, Some(page_url))}],
                }),
            )
            .await?;
        Ok(rows
            .iter()
            .filter_map(|r| {
                Some((
                    r["keys"][0].as_str()?.parse().ok()?,
                    r["impressions"].as_f64()? as i64,
                    r["clicks"].as_f64().unwrap_or(0.0) as i64,
                    r["position"].as_f64().unwrap_or(0.0),
                ))
            })
            .collect())
    }
}

fn gsc_terms(rows: &[Value], source: &str) -> Vec<Term> {
    rows.iter()
        .filter_map(|r| {
            Some(Term {
                term: r["keys"][0].as_str()?.to_string(),
                source: source.into(),
                impressions: r["impressions"].as_f64().map(|n| n as i64),
                clicks: r["clicks"].as_f64().map(|n| n as i64),
                position: r["position"].as_f64().map(|p| (p * 10.0).round() / 10.0),
            })
        })
        .collect()
}

#[async_trait]
impl DemandSource for SearchConsole {
    fn id(&self) -> &'static str {
        "gsc"
    }

    /// Queries this page already shows for in the market's country; when there are fewer than five, the site's
    /// top queries in that country as well (`gsc_site`: the LLM keeps the ones the page answers).
    async fn terms(&self, q: &Query<'_>) -> Result<Vec<Term>, String> {
        let access = self.google.access_token(&self.refresh_token).await?;
        let (start, end) = Self::window(28);
        let body = |filters: Vec<Value>| {
            let mut b = json!({"startDate": start, "endDate": end, "dimensions": ["query"], "rowLimit": 25});
            if !filters.is_empty() {
                b["dimensionFilterGroups"] = json!([{"filters": filters}]);
            }
            b
        };
        let mut out = match q.page_url {
            Some(u) => gsc_terms(
                &self
                    .query(&access, body(Self::filters(q.market, Some(u))))
                    .await?,
                "gsc",
            ),
            None => vec![],
        };
        if out.len() < 5 && q.market.country.as_deref().and_then(alpha3).is_some() {
            out.extend(gsc_terms(
                &self
                    .query(&access, body(Self::filters(q.market, None)))
                    .await?,
                "gsc_site",
            ));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn property_and_country() {
        let props = vec![
            "https://cliponit.app/".to_string(),
            "sc-domain:cliponit.app".to_string(),
            "sc-domain:other.io".to_string(),
        ];
        assert_eq!(
            pick_property(&props, "https://es.cliponit.app").as_deref(),
            Some("sc-domain:cliponit.app")
        );
        assert_eq!(
            pick_property(&props[..1], "https://cliponit.app").as_deref(),
            Some("https://cliponit.app/")
        );
        assert_eq!(pick_property(&props[..1], "https://es.cliponit.app"), None);
        assert_eq!(pick_property(&props, "https://evil-cliponit.app"), None);
        assert_eq!(alpha3("mx"), Some("mex"));
        assert_eq!(alpha3("XX"), None);
    }

    #[tokio::test]
    async fn sources_answer_per_market() {
        let src = MockDemand(BTreeMap::from([(
            "es-MX".to_string(),
            vec![("cortar video en línea".to_string(), 8100)],
        )]));
        let ask = async |locale: &str| {
            let m = crate::local_seo::validate::market(locale);
            src.terms(&Query {
                market: &m,
                path: "/",
                page_url: None,
            })
            .await
            .unwrap()
        };
        assert_eq!(ask("es-MX").await[0].term, "cortar video en línea");
        assert!(ask("es-ES").await.is_empty());
    }

    #[test]
    fn ranking_is_by_volume_then_stable() {
        let t = |term: &str, n: Option<i64>| Term {
            term: term.into(),
            source: "manual".into(),
            impressions: n,
            clicks: None,
            position: None,
        };
        let r = rank(vec![
            t("b", None),
            t("a", None),
            t("c", Some(10)),
            t("C", Some(5)),
            t("d", Some(50)),
        ]);
        assert_eq!(
            r.iter().map(|t| t.term.as_str()).collect::<Vec<_>>(),
            ["d", "c", "a", "b"]
        );
    }
}
