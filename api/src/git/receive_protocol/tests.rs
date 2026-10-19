use super::*;

const OLD: &str = "1111111111111111111111111111111111111111";
const NEW: &str = "2222222222222222222222222222222222222222";
const ZERO: &str = "0000000000000000000000000000000000000000";

fn command(refname: &str, capabilities: bool) -> Vec<u8> {
    let payload = if capabilities {
        format!("{OLD} {NEW} {refname}\0report-status side-band-64k\n")
    } else {
        format!("{OLD} {NEW} {refname}\n")
    };
    pkt_line(payload.as_bytes())
}

#[test]
fn command_refs_waits_for_the_flush_and_skips_shallow_lines() {
    let mut buffer = pkt_line(format!("shallow {OLD}\n").as_bytes());
    buffer.extend(command("refs/heads/agent-1", true));
    buffer.extend(command("refs/heads/agent-2", false));
    assert_eq!(command_refs(&buffer).unwrap(), None);
    assert_eq!(
        command_refs(&[buffer.as_slice(), b"00"].concat()).unwrap(),
        None
    );
    buffer.extend_from_slice(FLUSH);
    buffer.extend_from_slice(b"PACK");
    assert_eq!(
        command_refs(&buffer).unwrap(),
        Some(vec![
            "refs/heads/agent-1".to_string(),
            "refs/heads/agent-2".to_string()
        ])
    );
}

#[test]
fn command_refs_rejects_malformed_lines() {
    assert!(command_refs(b"zzzz").is_err());
    assert!(command_refs(b"0004").is_err());
    assert!(command_refs(&pkt_line(b"not-a-command\n")).is_err());
}

#[tokio::test]
async fn reading_commands_replays_the_whole_body() {
    let mut first = command("refs/heads/agent-1", true);
    let split = first.split_off(20);
    let mut rest = split;
    rest.extend_from_slice(FLUSH);
    rest.extend_from_slice(b"PACK-data-follows");
    let expected = [first.clone(), rest.clone()].concat();
    let body = Body::from_stream(stream::iter([
        Ok::<_, std::io::Error>(Bytes::from(first)),
        Ok(Bytes::from(rest)),
    ]));

    let commands = read_receive_pack_commands(body).await.unwrap();

    assert_eq!(commands.refs, vec!["refs/heads/agent-1".to_string()]);
    let replayed = axum::body::to_bytes(commands.body, usize::MAX)
        .await
        .unwrap();
    assert_eq!(replayed.as_ref(), expected.as_slice());
}

#[tokio::test]
async fn reading_commands_rejects_a_body_without_a_flush() {
    let body = Body::from(command("refs/heads/agent-1", true));
    assert!(read_receive_pack_commands(body).await.is_err());
}

fn advertisement(refs: &[(&str, &str)]) -> Vec<u8> {
    let mut output = pkt_line(b"# service=git-receive-pack\n");
    output.extend_from_slice(FLUSH);
    for (index, (oid, refname)) in refs.iter().enumerate() {
        let payload = if index == 0 {
            format!("{oid} {refname}\0report-status delete-refs\n")
        } else {
            format!("{oid} {refname}\n")
        };
        output.extend(pkt_line(payload.as_bytes()));
    }
    output.extend_from_slice(FLUSH);
    output
}

#[test]
fn additional_refs_follow_the_advertised_refs() {
    let base = advertisement(&[(OLD, "refs/heads/main")]);
    let extra = vec![("refs/heads/agent-1".to_string(), NEW.to_string())];

    let output = advertise_additional_refs(&base, &extra).unwrap();

    assert_eq!(
        output,
        advertisement(&[(OLD, "refs/heads/main"), (NEW, "refs/heads/agent-1")])
    );
}

#[test]
fn additional_refs_replace_an_empty_advertisement() {
    let base = advertisement(&[(ZERO, NO_REFS)]);
    let extra = vec![
        ("refs/heads/agent-1".to_string(), NEW.to_string()),
        ("refs/heads/agent-2".to_string(), OLD.to_string()),
    ];

    let output = advertise_additional_refs(&base, &extra).unwrap();

    assert_eq!(
        output,
        advertisement(&[(NEW, "refs/heads/agent-1"), (OLD, "refs/heads/agent-2")])
    );
}

#[test]
fn no_additional_refs_leaves_the_advertisement_unchanged() {
    let base = advertisement(&[(OLD, "refs/heads/main")]);
    assert_eq!(advertise_additional_refs(&base, &[]).unwrap(), base);
}
