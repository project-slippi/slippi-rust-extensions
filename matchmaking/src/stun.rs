//! Just enough of STUN (RFC 5389) to learn the public address of a UDP socket:
//! building a binding request and reading the mapped address out of the
//! response. The socket I/O happens on the Dolphin side through ENet, since the
//! socket that asks must be the socket that later plays.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::atomic::{AtomicU64, Ordering};

const MAGIC_COOKIE: u32 = 0x2112_A442;
const BINDING_REQUEST: u16 = 0x0001;
const BINDING_SUCCESS: u16 = 0x0101;
const ATTR_MAPPED_ADDRESS: u16 = 0x0001;
const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
const FAMILY_IPV4: u8 = 0x01;

/// Size of a binding request with no attributes.
pub const REQUEST_LEN: usize = 20;

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Fills `out` with a binding request carrying a fresh transaction ID.
pub fn build_binding_request(out: &mut [u8; REQUEST_LEN]) {
    out[0..2].copy_from_slice(&BINDING_REQUEST.to_be_bytes());
    out[2..4].copy_from_slice(&0u16.to_be_bytes());
    out[4..8].copy_from_slice(&MAGIC_COOKIE.to_be_bytes());

    // The transaction ID only needs to be unpredictable enough that a stale
    // or foreign response is not mistaken for ours. The process-random hash
    // keys in RandomState give that without another dependency.
    let state = RandomState::new();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut a = state.build_hasher();
    a.write_u64(n);
    let mut b = state.build_hasher();
    b.write_u64(n ^ 0x9E37_79B9_7F4A_7C15);
    let mut id = [0u8; 16];
    id[..8].copy_from_slice(&a.finish().to_be_bytes());
    id[8..].copy_from_slice(&b.finish().to_be_bytes());
    out[8..20].copy_from_slice(&id[..12]);
}

/// Extracts the mapped address from a binding response to `request`. Returns
/// None if the bytes are not a success response to that request or carry no
/// IPv4 mapped address.
pub fn parse_binding_response(request: &[u8], response: &[u8]) -> Option<SocketAddrV4> {
    if request.len() < REQUEST_LEN || response.len() < REQUEST_LEN {
        return None;
    }

    let msg_type = u16::from_be_bytes([response[0], response[1]]);
    if msg_type != BINDING_SUCCESS {
        return None;
    }
    let length = u16::from_be_bytes([response[2], response[3]]) as usize;
    if response[4..8] != MAGIC_COOKIE.to_be_bytes() || response[8..20] != request[8..20] {
        return None;
    }
    let body = &response[20..response.len().min(20 + length)];

    let mut fallback = None;
    let mut offset = 0;
    while offset + 4 <= body.len() {
        let attr_type = u16::from_be_bytes([body[offset], body[offset + 1]]);
        let attr_len = u16::from_be_bytes([body[offset + 2], body[offset + 3]]) as usize;
        let value_start = offset + 4;
        let value_end = value_start + attr_len;
        if value_end > body.len() {
            break;
        }
        let value = &body[value_start..value_end];

        match attr_type {
            ATTR_XOR_MAPPED_ADDRESS => {
                if let Some(addr) = decode_address(value, true) {
                    return Some(addr);
                }
            },
            ATTR_MAPPED_ADDRESS => {
                if fallback.is_none() {
                    fallback = decode_address(value, false);
                }
            },
            _ => {},
        }

        // Attributes are padded to four-byte boundaries.
        offset = value_end + ((4 - (attr_len % 4)) % 4);
    }

    fallback
}

fn decode_address(value: &[u8], xored: bool) -> Option<SocketAddrV4> {
    if value.len() < 8 || value[1] != FAMILY_IPV4 {
        return None;
    }
    let mut port = u16::from_be_bytes([value[2], value[3]]);
    let mut ip = u32::from_be_bytes([value[4], value[5], value[6], value[7]]);
    if xored {
        port ^= (MAGIC_COOKIE >> 16) as u16;
        ip ^= MAGIC_COOKIE;
    }
    Some(SocketAddrV4::new(Ipv4Addr::from(ip), port))
}

/// What two binding results from different servers say about the NAT.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NatType {
    /// One or both lookups failed.
    Unknown,
    /// The same public port was seen by both servers, so other peers can reach
    /// it too.
    Cone,
    /// Each destination got its own public port, so peers cannot predict it.
    Symmetric,
}

impl NatType {
    /// The string the matchmaking service expects.
    pub fn as_str(self) -> &'static str {
        match self {
            NatType::Unknown => "unknown",
            NatType::Cone => "cone",
            NatType::Symmetric => "symmetric",
        }
    }
}

/// Classifies the NAT from the mapped addresses two different STUN servers
/// reported for the same local socket.
pub fn classify_nat(first: Option<SocketAddrV4>, second: Option<SocketAddrV4>) -> NatType {
    match (first, second) {
        (Some(a), Some(b)) if a == b => NatType::Cone,
        (Some(_), Some(_)) => NatType::Symmetric,
        _ => NatType::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn response_for(request: &[u8; REQUEST_LEN], attrs: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&BINDING_SUCCESS.to_be_bytes());
        out.extend_from_slice(&(attrs.len() as u16).to_be_bytes());
        out.extend_from_slice(&MAGIC_COOKIE.to_be_bytes());
        out.extend_from_slice(&request[8..20]);
        out.extend_from_slice(attrs);
        out
    }

    fn xor_mapped(ip: Ipv4Addr, port: u16) -> Vec<u8> {
        let mut attr = vec![0x00, 0x20, 0x00, 0x08, 0x00, FAMILY_IPV4];
        attr.extend_from_slice(&(port ^ (MAGIC_COOKIE >> 16) as u16).to_be_bytes());
        attr.extend_from_slice(&(u32::from(ip) ^ MAGIC_COOKIE).to_be_bytes());
        attr
    }

    #[test]
    fn request_has_header_and_unique_ids() {
        let mut a = [0u8; REQUEST_LEN];
        let mut b = [0u8; REQUEST_LEN];
        build_binding_request(&mut a);
        build_binding_request(&mut b);
        assert_eq!(&a[0..4], &[0x00, 0x01, 0x00, 0x00]);
        assert_eq!(&a[4..8], &MAGIC_COOKIE.to_be_bytes());
        assert_ne!(&a[8..20], &b[8..20]);
    }

    #[test]
    fn parses_xor_mapped_address() {
        let mut req = [0u8; REQUEST_LEN];
        build_binding_request(&mut req);
        let ip = Ipv4Addr::new(203, 0, 113, 7);
        let resp = response_for(&req, &xor_mapped(ip, 41234));
        assert_eq!(parse_binding_response(&req, &resp), Some(SocketAddrV4::new(ip, 41234)));
    }

    #[test]
    fn prefers_xor_mapped_over_mapped_and_skips_unknown_attributes() {
        let mut req = [0u8; REQUEST_LEN];
        build_binding_request(&mut req);

        // SOFTWARE attribute with odd length to exercise padding.
        let mut attrs = vec![0x80, 0x22, 0x00, 0x05, b'h', b'e', b'l', b'l', b'o', 0, 0, 0];
        // Plain MAPPED-ADDRESS pointing somewhere else.
        attrs.extend_from_slice(&[0x00, 0x01, 0x00, 0x08, 0x00, FAMILY_IPV4]);
        attrs.extend_from_slice(&1000u16.to_be_bytes());
        attrs.extend_from_slice(&u32::from(Ipv4Addr::new(10, 0, 0, 1)).to_be_bytes());
        let ip = Ipv4Addr::new(198, 51, 100, 2);
        attrs.extend_from_slice(&xor_mapped(ip, 5000));

        let resp = response_for(&req, &attrs);
        assert_eq!(parse_binding_response(&req, &resp), Some(SocketAddrV4::new(ip, 5000)));
    }

    #[test]
    fn rejects_wrong_transaction_or_type() {
        let mut req = [0u8; REQUEST_LEN];
        build_binding_request(&mut req);
        let mut other = [0u8; REQUEST_LEN];
        build_binding_request(&mut other);
        let ip = Ipv4Addr::new(203, 0, 113, 7);

        let foreign = response_for(&other, &xor_mapped(ip, 1));
        assert_eq!(parse_binding_response(&req, &foreign), None);

        let mut error = response_for(&req, &xor_mapped(ip, 1));
        error[0..2].copy_from_slice(&0x0111u16.to_be_bytes());
        assert_eq!(parse_binding_response(&req, &error), None);

        assert_eq!(parse_binding_response(&req, &[0u8; 5]), None);
    }

    #[test]
    fn classifies_nat() {
        let a = SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 40000);
        let b = SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 40001);
        assert_eq!(classify_nat(Some(a), Some(a)), NatType::Cone);
        assert_eq!(classify_nat(Some(a), Some(b)), NatType::Symmetric);
        assert_eq!(classify_nat(Some(a), None), NatType::Unknown);
        assert_eq!(classify_nat(None, None), NatType::Unknown);
    }
}
