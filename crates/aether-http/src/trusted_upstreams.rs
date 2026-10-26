use std::collections::HashSet;
use std::net::IpAddr;

/// Administrator-owned exceptions for literal upstream URLs. Deliberately has
/// no hostname/DNS matching, CIDRs, wildcards, or implicit ports.
#[derive(Debug, Default)]
pub struct TrustedUpstreamOrigins(HashSet<(String, IpAddr, u16)>);

impl TrustedUpstreamOrigins {
    pub fn parse(value: &str) -> Result<Self, &'static str> {
        let mut origins = HashSet::new();
        if value.trim().is_empty() {
            return Ok(Self(origins));
        }
        for entry in value.split(',') {
            let entry = entry.trim();
            let url = url::Url::parse(entry).map_err(|_| "invalid trusted upstream origin")?;
            let ip =
                literal_ip(&url).ok_or("trusted upstream origins require literal IP addresses")?;
            let port = url
                .port_or_known_default()
                .ok_or("invalid trusted upstream port")?;
            // Checking the serialized authority also requires an explicit port
            // for HTTP/HTTPS defaults, which URL parsing otherwise normalizes away.
            let authority = entry
                .split_once("://")
                .map(|(_, rest)| rest.trim_end_matches('/'));
            let expected = match ip {
                IpAddr::V4(ip) => format!("{ip}:{port}"),
                IpAddr::V6(ip) => format!("[{ip}]:{port}"),
            };
            if !matches!(url.scheme(), "http" | "https")
                || authority != Some(expected.as_str())
                || port == 0
                || !url.username().is_empty()
                || url.password().is_some()
                || url.path() != "/"
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err("trusted upstream origins must be http(s)://IP:port without credentials, path, query or fragment");
            }
            origins.insert((url.scheme().to_owned(), ip, port));
        }
        Ok(Self(origins))
    }

    pub fn allows(&self, url: &url::Url) -> bool {
        if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
            return false;
        }
        literal_ip(url)
            .zip(url.port_or_known_default())
            .is_some_and(|(ip, port)| self.0.contains(&(url.scheme().to_owned(), ip, port)))
    }
}

fn literal_ip(url: &url::Url) -> Option<IpAddr> {
    match url.host()? {
        url::Host::Ipv4(ip) => Some(IpAddr::V4(ip)),
        url::Host::Ipv6(ip) => Some(IpAddr::V6(ip)),
        url::Host::Domain(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trusted_upstreams_match_only_exact_literal_origins() {
        let policy =
            TrustedUpstreamOrigins::parse("http://100.64.0.6:65500, https://[fd00::6]:443")
                .unwrap();
        for target in [
            "http://100.64.0.6:65500/models",
            "http://100.64.0.6:65500/v1/messages?x=1",
            "https://[fd00::6]/v1/models",
        ] {
            assert!(policy.allows(&url::Url::parse(target).unwrap()), "{target}");
        }
        for target in [
            "http://100.64.0.7:65500/v1/models",
            "http://100.64.0.6:65501/v1/models",
            "https://100.64.0.6:65500/v1/models",
            "http://100.64.0.6/v1/models",
            "http://magpie.local:65500/v1/models",
            "http://100.64.0.6.example.com:65500/v1/models",
            "http://[::ffff:100.64.0.6]:65500/v1/models",
            "http://secret@100.64.0.6:65500/v1/models",
            "http://100.64.0.6:65500/v1/models#secret",
        ] {
            assert!(
                !policy.allows(&url::Url::parse(target).unwrap()),
                "{target}"
            );
        }
        assert!(!TrustedUpstreamOrigins::default()
            .allows(&url::Url::parse("http://100.64.0.6:65500").unwrap()));
    }

    #[test]
    fn trusted_upstreams_reject_ambiguous_or_overbroad_configuration() {
        for value in [
            "100.64.0.6:65500",
            "http://100.64.0.6",
            "http://100.64.0.0/10:65500",
            "http://*.local:65500",
            "http://magpie.local:65500",
            "http://100.64.0.6:0",
            "http://secret@100.64.0.6:65500",
            "http://100.64.0.6:65500/v1",
            "http://100.64.0.6:65500?secret",
            "http://100.64.0.6:65500#secret",
            "ftp://100.64.0.6:65500",
            "http://100.64.0.6:65500,",
        ] {
            assert!(TrustedUpstreamOrigins::parse(value).is_err(), "{value}");
        }
        assert!(TrustedUpstreamOrigins::parse("").is_ok());
    }
}
