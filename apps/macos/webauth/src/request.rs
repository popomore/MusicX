use anyhow::{Result, ensure};
use musicx_protocol::tr;
use serde::Deserialize;
use url::Url;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Request {
    pub url: String,
    pub cookies_of: String,
    pub until: Vec<String>,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default)]
    pub name: String,
}
impl Request {
    pub fn validate(&self) -> Result<()> {
        for address in [&self.url, &self.cookies_of] {
            let url = Url::parse(address)?;
            ensure!(
                address.len() <= 8192
                    && url.scheme() == "https"
                    && url.username().is_empty()
                    && url.password().is_none()
                    && url.domain().is_some_and(|h| h.contains('.')
                        && !h.ends_with(".local")
                        && !h.ends_with(".localhost"))
                    && url.port_or_known_default() == Some(443),
                tr!("无效的网页登录地址")
            );
        }
        ensure!(
            !self.until.is_empty()
                && self.until.len() <= 16
                && self.until.iter().all(|s| !s.is_empty()
                    && s.len() <= 64
                    && s.bytes()
                        .all(|c| c.is_ascii_alphanumeric() || b"_-".contains(&c))),
            tr!("无效的登录完成条件")
        );
        Ok(())
    }
    pub fn accepts_cookie(&self, domain: &str, path: &str) -> bool {
        let Ok(url) = Url::parse(&self.cookies_of) else {
            return false;
        };
        let Some(host) = url.domain() else {
            return false;
        };
        let domain = domain.trim_start_matches('.').to_ascii_lowercase();
        let domain_match =
            host == domain || host.strip_suffix(&domain).is_some_and(|p| p.ends_with('.'));
        let request_path = url.path();
        let path_match = request_path == path
            || request_path
                .strip_prefix(path)
                .is_some_and(|rest| path.ends_with('/') || rest.starts_with('/'));
        domain_match && path_match
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cookies_stay_with_the_requested_host_and_path() {
        let r = Request {
            url: "https://music.163.com/#/login".into(),
            cookies_of: "https://music.163.com/".into(),
            until: vec!["MUSIC_U".into()],
            user_agent: None,
            name: String::new(),
        };
        assert!(r.validate().is_ok());
        assert!(r.accepts_cookie(".music.163.com", "/"));
        assert!(r.accepts_cookie(".163.com", "/"));
        assert!(!r.accepts_cookie("evil163.com", "/"));
        assert!(!r.accepts_cookie("accounts.163.com", "/"));
        assert!(!r.accepts_cookie("music.163.com", "/private"));
        let mut invalid = r;
        invalid.url = "http://music.163.com/".into();
        assert!(invalid.validate().is_err());
    }
}
