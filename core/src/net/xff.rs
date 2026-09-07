use anyhow::{bail, Result};

pub fn apply_x_forwarded_for(head: &mut Vec<u8>, client_ip: &str, proto: &str) -> Result<()> {
    let text = String::from_utf8_lossy(head);
    let mut lines: Vec<String> = text.split_inclusive('\n').map(|s| s.to_string()).collect();
    if lines.is_empty() {
        bail!("empty http request");
    }

    let ending = if lines.iter().any(|l| l.ends_with("\r\n")) {
        "\r\n"
    } else {
        "\n"
    };

    let blank_idx = lines
        .iter()
        .position(|l| l == "\r\n" || l == "\n")
        .unwrap_or(lines.len());

    let mut xff_idx: Option<usize> = None;
    let mut has_proto = false;
    for (i, line) in lines.iter().enumerate().take(blank_idx).skip(1) {
        let trimmed = line.trim_start_matches([' ', '\t']);
        let lower_ok_xff = trimmed.len() >= 16
            && trimmed.as_bytes()[..15].eq_ignore_ascii_case(b"x-forwarded-for")
            && trimmed.as_bytes().get(15) == Some(&b':');
        if lower_ok_xff {
            xff_idx = Some(i);
        }
        let lower_ok_proto = trimmed.len() >= 18
            && trimmed.as_bytes()[..17].eq_ignore_ascii_case(b"x-forwarded-proto")
            && trimmed.as_bytes().get(17) == Some(&b':');
        if lower_ok_proto {
            has_proto = true;
        }
    }

    if !client_ip.is_empty() {
        if let Some(i) = xff_idx {
            let line = &lines[i];
            let trimmed = line.trim_start_matches([' ', '\t']);
            let colon = trimmed
                .find(':')
                .ok_or_else(|| anyhow::anyhow!("malformed X-Forwarded-For"))?;
            let value = trimmed[colon + 1..].trim().trim_end_matches(['\r', '\n']);
            let new_val = if value.is_empty() {
                client_ip.to_string()
            } else {
                format!("{value}, {client_ip}")
            };
            lines[i] = format!("X-Forwarded-For: {new_val}{ending}");
        } else {
            lines.insert(blank_idx, format!("X-Forwarded-For: {client_ip}{ending}"));
        }
    }

    if !proto.is_empty() && !has_proto {
        let blank_idx = lines
            .iter()
            .position(|l| l == "\r\n" || l == "\n")
            .unwrap_or(lines.len());
        lines.insert(blank_idx, format!("X-Forwarded-Proto: {proto}{ending}"));
    }

    *head = lines.join("").into_bytes();
    Ok(())
}
