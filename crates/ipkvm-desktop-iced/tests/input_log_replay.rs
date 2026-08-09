use std::collections::BTreeMap;

type Fields<'a> = BTreeMap<&'a str, &'a str>;

fn parse_logfmt(line: &str) -> Fields<'_> {
    let mut fields = BTreeMap::new();
    let bytes = line.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let key_start = index;
        while index < bytes.len() && bytes[index] != b'=' {
            index += 1;
        }
        if index >= bytes.len() {
            break;
        }
        let key = &line[key_start..index];
        index += 1;
        let value_start = index;
        let value = if index < bytes.len() && bytes[index] == b'"' {
            index += 1;
            let inner_start = index;
            while index < bytes.len() && bytes[index] != b'"' {
                index += 1;
            }
            let value = &line[inner_start..index];
            if index < bytes.len() {
                index += 1;
            }
            value
        } else {
            while index < bytes.len() && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            &line[value_start..index]
        };
        fields.insert(key, value);
    }
    fields
}

fn field_u64(fields: &Fields<'_>, key: &str) -> u64 {
    fields
        .get(key)
        .unwrap_or_else(|| panic!("missing {key} in {fields:?}"))
        .parse()
        .unwrap_or_else(|_| panic!("invalid {key} in {fields:?}"))
}

#[test]
fn macos_absolute_drag_log_keeps_left_button_through_output_chain() {
    let log = include_str!("fixtures/macos_absolute_drag_diag.logfmt");
    let entries: Vec<_> = log
        .lines()
        .enumerate()
        .map(|(index, line)| {
            let fields = parse_logfmt(line);
            assert!(
                fields.contains_key("component")
                    && fields.contains_key("event")
                    && fields.contains_key("mono_ms"),
                "fixture line {} is missing required fields: {line}",
                index + 1
            );
            fields
        })
        .collect();

    let start = entries
        .iter()
        .find(|fields| {
            fields.get("component") == Some(&"desktop.iced")
                && fields.get("event") == Some(&"mouse_button")
                && fields.get("action") == Some(&"pressed")
                && fields.get("button") == Some(&"left")
                && fields.get("mask_after") == Some(&"0x01")
        })
        .map(|fields| field_u64(fields, "mono_ms"))
        .expect("drag press not found");
    let end = entries
        .iter()
        .find(|fields| {
            fields.get("component") == Some(&"desktop.iced")
                && fields.get("event") == Some(&"mouse_button")
                && fields.get("action") == Some(&"released")
                && fields.get("button") == Some(&"left")
                && fields.get("mask_before") == Some(&"0x01")
                && fields.get("mask_after") == Some(&"0x00")
        })
        .map(|fields| field_u64(fields, "mono_ms"))
        .expect("drag release not found");

    assert!(
        end - start >= 1_000,
        "fixture must represent a long drag, got {} ms",
        end - start
    );

    let mut desktop_sent = 0;
    let mut mapper_seen = 0;
    let mut mapper_committed_pressed = 0;
    let mut ch9329_move_reports = 0;
    let mut serial_pressed_frames = 0;

    for fields in &entries {
        let mono_ms = field_u64(fields, "mono_ms");
        let component = fields.get("component").copied();
        let event = fields.get("event").copied();
        let within_drag = mono_ms >= start && mono_ms <= end;
        if !within_drag {
            continue;
        }

        if component == Some("desktop.iced")
            && event == Some("absolute_pointer")
            && fields.get("decision") == Some(&"sent")
            && mono_ms < end
        {
            desktop_sent += 1;
            assert_eq!(fields.get("mask"), Some(&"0x01"), "{fields:?}");
        }

        if component == Some("session.rfb_pointer")
            && event == Some("absolute_map")
            && mono_ms < end
        {
            mapper_seen += 1;
            assert_eq!(fields.get("incoming_mask"), Some(&"0x01"), "{fields:?}");
            if fields.get("committed_mask") == Some(&"0x01") {
                mapper_committed_pressed += 1;
            }
        }

        if component == Some("core.ch9329")
            && event == Some("pointer_report")
            && fields.get("report") == Some(&"absolute")
            && mono_ms > start
            && mono_ms < end
        {
            ch9329_move_reports += 1;
            assert_eq!(fields.get("buttons"), Some(&"0x01"), "{fields:?}");
        }

        if component == Some("core.serial")
            && event == Some("frame_tx")
            && fields.get("command") == Some(&"0x04")
            && mono_ms > start
            && mono_ms < end
        {
            let summary = fields.get("summary").copied().unwrap_or_default();
            assert!(summary.contains("buttons=0x01"), "{fields:?}");
            serial_pressed_frames += 1;
        }
    }

    assert!(desktop_sent >= 3, "desktop sent count={desktop_sent}");
    assert!(mapper_seen >= 3, "mapper seen count={mapper_seen}");
    assert!(
        mapper_committed_pressed >= 2,
        "mapper committed pressed count={mapper_committed_pressed}"
    );
    assert!(
        ch9329_move_reports >= 2,
        "ch9329 report count={ch9329_move_reports}"
    );
    assert!(
        serial_pressed_frames >= 2,
        "serial pressed frame count={serial_pressed_frames}"
    );
}
