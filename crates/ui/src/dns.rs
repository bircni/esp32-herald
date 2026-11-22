//! The captive portal's DNS: in setup mode every name resolves to the
//! device, so a phone joining its network opens the setup page by itself.

use std::net::Ipv4Addr;

const HEADER: usize = 12;
const TYPE_A: u16 = 1;
const CLASS_IN: u16 = 1;
const TTL_S: u32 = 60;

/// The reply to the DNS `query`: an `A` record with `ip` for `A` questions,
/// an empty answer for other types (e.g. `AAAA`). `None` for anything that
/// is not a well-formed standard query.
#[must_use]
pub fn answer(query: &[u8], ip: Ipv4Addr) -> Option<Vec<u8>> {
    let header = query.get(..HEADER)?;
    let flags = u16::from_be_bytes([header[2], header[3]]);
    let questions = u16::from_be_bytes([header[4], header[5]]);
    // Only standard queries (QR = 0, opcode 0) with one question.
    if flags & 0xF800 != 0 || questions != 1 {
        return None;
    }
    // The question: a name of length-prefixed labels, then type and class.
    let mut i = HEADER;
    loop {
        let len = usize::from(*query.get(i)?);
        if len == 0 {
            i += 1;
            break;
        }
        // Compression pointers do not belong in a question.
        if len & 0xC0 != 0 {
            return None;
        }
        i += 1 + len;
    }
    let qtype = u16::from_be_bytes([*query.get(i)?, *query.get(i + 1)?]);
    let question_end = i + 4;
    let question = query.get(HEADER..question_end)?;
    let is_a = qtype == TYPE_A;

    let mut reply = Vec::with_capacity(question_end + 16);
    reply.extend_from_slice(&header[..2]); // id
    // Response, authoritative, recursion desired copied, recursion available.
    let rd = header[2] & 0x01;
    reply.extend_from_slice(&[0x84 | rd, 0x80]);
    reply.extend_from_slice(&1u16.to_be_bytes()); // questions
    reply.extend_from_slice(&u16::from(is_a).to_be_bytes()); // answers
    reply.extend_from_slice(&[0, 0, 0, 0]); // authority, additional
    reply.extend_from_slice(question);
    if is_a {
        reply.extend_from_slice(&[0xC0, HEADER as u8]); // name: pointer to the question
        reply.extend_from_slice(&TYPE_A.to_be_bytes());
        reply.extend_from_slice(&CLASS_IN.to_be_bytes());
        reply.extend_from_slice(&TTL_S.to_be_bytes());
        reply.extend_from_slice(&4u16.to_be_bytes());
        reply.extend_from_slice(&ip.octets());
    }
    Some(reply)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A query for `name` of type `qtype`, as a phone sends it.
    fn query(name: &str, qtype: u16) -> Vec<u8> {
        let mut q = vec![0x12, 0x34, 0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0];
        for label in name.split('.') {
            q.push(label.len() as u8);
            q.extend_from_slice(label.as_bytes());
        }
        q.push(0);
        q.extend_from_slice(&qtype.to_be_bytes());
        q.extend_from_slice(&CLASS_IN.to_be_bytes());
        q
    }

    #[test]
    fn every_name_points_to_the_device() {
        let ip = Ipv4Addr::new(192, 168, 71, 1);
        let q = query("connectivitycheck.gstatic.com", TYPE_A);
        let r = answer(&q, ip).unwrap();
        assert_eq!(&r[..2], &[0x12, 0x34], "same id");
        assert_eq!(r[2] & 0x80, 0x80, "a response");
        assert_eq!(u16::from_be_bytes([r[6], r[7]]), 1, "one answer");
        assert_eq!(&r[r.len() - 4..], &[192, 168, 71, 1]);
        assert_eq!(&r[12..q.len()], &q[12..], "the question is repeated");
    }

    #[test]
    fn other_types_get_no_records_and_garbage_no_reply() {
        let ip = Ipv4Addr::new(10, 0, 0, 1);
        let r = answer(&query("example.com", 28), ip).unwrap();
        assert_eq!(u16::from_be_bytes([r[6], r[7]]), 0, "no AAAA record");
        assert!(answer(&[1, 2, 3], ip).is_none(), "too short");
        let mut response = query("a.b", TYPE_A);
        response[2] = 0x80;
        assert!(answer(&response, ip).is_none(), "responses are ignored");
        let mut cut = query("example.com", TYPE_A);
        cut.truncate(cut.len() - 3);
        assert!(answer(&cut, ip).is_none(), "truncated question");
    }
}
