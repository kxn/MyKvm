use ipkvm_rfb::{
    BgraFrameView, FramebufferUpdateRequest, RfbConnectionConfig, RfbConnectionCore, RfbEvent,
    RfbProtocolLimits, RfbRectangle, RfbSize,
};

#[test]
fn completes_handshake_negotiation_request_and_raw_update() {
    let size = RfbSize::new(2, 1).unwrap();
    let config = RfbConnectionConfig {
        desktop_name: "my_ipkvm".to_owned(),
        initial_size: size,
        limits: RfbProtocolLimits::default(),
    };
    let mut connection = RfbConnectionCore::new(config).unwrap();

    assert_eq!(connection.take_output(), b"RFB 003.008\n");
    assert!(connection.push_input(b"RFB 003.008\n").is_empty());
    assert_eq!(connection.take_output(), [1, 1]);
    assert!(connection.push_input(&[1]).is_empty());
    assert_eq!(connection.take_output(), [0, 0, 0, 0]);
    assert_eq!(
        connection.push_input(&[1]),
        vec![Ok(RfbEvent::HandshakeCompleted { shared: true })]
    );
    assert!(!connection.take_output().is_empty());

    let mut client_messages = vec![2, 0, 0, 2];
    client_messages.extend_from_slice(&0_i32.to_be_bytes());
    client_messages.extend_from_slice(&(-223_i32).to_be_bytes());
    client_messages.extend_from_slice(&[3, 0, 0, 0, 0, 0, 0, 2, 0, 1]);
    assert_eq!(
        connection.push_input(&client_messages),
        vec![Ok(RfbEvent::FramebufferUpdateRequested(
            FramebufferUpdateRequest {
                incremental: false,
                rectangle: RfbRectangle {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
            }
        ))]
    );

    let pixels = [1, 2, 3, 255, 4, 5, 6, 255];
    let frame = BgraFrameView::new(size, 8, &pixels).unwrap();
    connection
        .queue_framebuffer_update(
            frame,
            FramebufferUpdateRequest {
                incremental: false,
                rectangle: RfbRectangle {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
            },
        )
        .unwrap();
    assert_eq!(
        connection.take_output(),
        [
            0, 0, 0, 1, 0, 0, 0, 0, 0, 2, 0, 1, 0, 0, 0, 0, 1, 2, 3, 0, 4, 5, 6, 0,
        ]
    );
}
