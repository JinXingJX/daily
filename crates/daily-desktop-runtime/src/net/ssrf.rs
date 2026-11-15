use reqwest::Url;
use std::net::{IpAddr, Ipv4Addr};

pub(crate) fn validate_plugin_network_url(url: &Url) -> Result<(), String> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("network.fetch only supports http and https URLs.".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("network.fetch URLs cannot contain embedded credentials.".into());
    }
    let host = url
        .host_str()
        .map(|value| value.trim_end_matches('.').to_ascii_lowercase())
        .ok_or_else(|| "network.fetch URL must include a host.".to_string())?;
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") {
        return Err("network.fetch cannot access local network hosts.".into());
    }
    let address_host = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(address) = address_host.parse::<IpAddr>() {
        if is_blocked_network_ip(address) {
            return Err("network.fetch cannot access private or local IP addresses.".into());
        }
    }
    Ok(())
}

pub(crate) fn is_blocked_network_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => is_blocked_ipv4(address),
        IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                return is_blocked_ipv4(mapped);
            }
            let segments = address.segments();
            let is_global_unicast = segments[0] & 0xe000 == 0x2000;
            let is_six_to_four = segments[0] == 0x2002;
            let embedded_v4 = Ipv4Addr::new(
                (segments[1] >> 8) as u8,
                segments[1] as u8,
                (segments[2] >> 8) as u8,
                segments[2] as u8,
            );
            !is_global_unicast || (is_six_to_four && is_blocked_ipv4(embedded_v4))
        }
    }
}

/// Like [`is_blocked_network_ip`], but for addresses returned by DNS. Proxy tools in
/// fake-ip mode (Clash, Surge, sing-box) answer every lookup from 198.18.0.0/15 and
/// route by hostname, so that range must stay reachable after resolution. Literal IPs
/// in URLs are still checked with the strict rule.
pub(crate) fn is_blocked_resolved_ip(address: IpAddr) -> bool {
    let v4 = match address {
        IpAddr::V4(address) => Some(address),
        IpAddr::V6(address) => address.to_ipv4_mapped(),
    };
    if let Some(v4) = v4 {
        let [first, second, ..] = v4.octets();
        if first == 198 && matches!(second, 18 | 19) {
            return false;
        }
    }
    is_blocked_network_ip(address)
}

pub(crate) fn is_blocked_ipv4(address: Ipv4Addr) -> bool {
    let [first, second, third, _fourth] = address.octets();
    first == 0
        || first == 10
        || first == 127
        || (first == 100 && (64..=127).contains(&second))
        || (first == 169 && second == 254)
        || (first == 172 && (16..=31).contains(&second))
        || (first == 192 && second == 0 && matches!(third, 0 | 2))
        || (first == 192 && second == 88 && third == 99)
        || (first == 192 && second == 168)
        || (first == 198 && matches!(second, 18 | 19))
        || (first == 198 && second == 51 && third == 100)
        || (first == 203 && second == 0 && third == 113)
        || first >= 224
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolved_fake_ip_range_is_allowed_but_literal_is_not() {
        let fake_ip: IpAddr = "198.18.0.123".parse().unwrap();
        assert!(!is_blocked_resolved_ip(fake_ip));
        assert!(is_blocked_network_ip(fake_ip));
        assert!(validate_plugin_network_url(&Url::parse("http://198.18.0.123/").unwrap()).is_err());
        assert!(is_blocked_resolved_ip("192.168.1.1".parse().unwrap()));
        assert!(is_blocked_resolved_ip("127.0.0.1".parse().unwrap()));
    }
}
