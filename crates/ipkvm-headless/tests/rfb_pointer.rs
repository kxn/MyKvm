use ipkvm_core::{
    Ch9329InputSink, CommandQueueError, FramebufferSize, InputError, InputResult, InputSink,
    KeyEvent, MouseMode, PointerEvent, fake_serial::FakeCommandQueue,
};
use ipkvm_headless::rfb_input::{RfbPointerError, RfbPointerMapper, RfbPointerOutcome};

#[derive(Default)]
struct RecordingSink {
    pointer_batches: Vec<Vec<PointerEvent>>,
}

impl InputSink for RecordingSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
        Ok(())
    }

    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
        Ok(())
    }

    fn handle_pointer_batch(&mut self, events: &[PointerEvent]) -> InputResult<()> {
        self.pointer_batches.push(events.to_vec());
        Ok(())
    }

    fn release_all(&mut self) -> InputResult<()> {
        Ok(())
    }
}

fn full_hd() -> FramebufferSize {
    FramebufferSize {
        width: 1920,
        height: 1080,
    }
}

#[test]
fn public_outcome_reports_unsupported_buttons_without_extra_events() {
    let mut mapper = RfbPointerMapper::new();
    let mut sink = RecordingSink::default();

    assert_eq!(
        mapper.handle_pointer(&mut sink, 0xe0, 10, 20, full_hd()),
        Ok(RfbPointerOutcome::AppliedIgnoringButtons { button_mask: 0xe0 })
    );
    assert_eq!(
        sink.pointer_batches,
        vec![vec![PointerEvent::AbsoluteMove {
            x: 10,
            y: 20,
            framebuffer_size: full_hd(),
        }]]
    );
}

#[test]
fn real_sink_maps_framebuffer_corners_with_vendor_formula() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let mut mapper = RfbPointerMapper::new();

    mapper
        .handle_pointer(&mut sink, 0, 0, 0, full_hd())
        .unwrap();
    mapper
        .handle_pointer(&mut sink, 0, 1919, 1079, full_hd())
        .unwrap();

    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].frames().len(), 1);
    assert_eq!(&batches[0].frames()[0].data()[2..6], &[0, 0, 0, 0]);
    assert_eq!(batches[1].frames().len(), 1);
    assert_eq!(
        &batches[1].frames()[0].data()[2..6],
        &[0xfd, 0x0f, 0xfc, 0x0f]
    );
}

#[test]
fn first_button_message_is_one_atomic_command_batch() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let mut mapper = RfbPointerMapper::new();

    mapper
        .handle_pointer(&mut sink, 0x01, 960, 540, full_hd())
        .unwrap();

    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].frames().len(), 2);
    let move_report = batches[0].frames()[0].data();
    let button_report = batches[0].frames()[1].data();
    assert_eq!(move_report[1], 0);
    assert_eq!(button_report[1], 1);
    assert_eq!(&move_report[2..6], &button_report[2..6]);
}

#[test]
fn queue_failure_rolls_back_mapper_and_real_sink_together() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let mut mapper = RfbPointerMapper::new();
    queue.fail_next(CommandQueueError::Closed);

    assert_eq!(
        mapper.handle_pointer(&mut sink, 0x09, 100, 200, full_hd()),
        Err(RfbPointerError::Input(InputError::CommandQueue(
            CommandQueueError::Closed
        )))
    );
    assert!(queue.accepted_batches().is_empty());

    mapper
        .handle_pointer(&mut sink, 0x09, 100, 200, full_hd())
        .unwrap();
    mapper
        .handle_pointer(&mut sink, 0x09, 101, 201, full_hd())
        .unwrap();

    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].frames().len(), 3);
    assert_eq!(batches[0].frames()[0].data()[1], 0);
    assert_eq!(batches[0].frames()[1].data()[1], 1);
    assert_eq!(batches[0].frames()[2].data()[6], 1);
    assert_eq!(batches[1].frames().len(), 1);
    assert_eq!(batches[1].frames()[0].data()[1], 1);
    assert_eq!(batches[1].frames()[0].data()[6], 0);
}

#[test]
fn out_of_bounds_message_does_not_commit_button_mask() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let mut mapper = RfbPointerMapper::new();

    assert_eq!(
        mapper.handle_pointer(&mut sink, 0x01, 1920, 100, full_hd()),
        Err(RfbPointerError::Input(InputError::PointerOutOfBounds {
            coordinate: 1920,
            extent: 1920,
        }))
    );
    assert!(queue.accepted_batches().is_empty());

    mapper
        .handle_pointer(&mut sink, 0x01, 1919, 100, full_hd())
        .unwrap();
    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 1);
    assert_eq!(batches[0].frames().len(), 2);
    assert_eq!(batches[0].frames()[1].data()[1], 1);
}

#[test]
fn zero_framebuffer_size_is_rejected_without_queue_activity() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let mut mapper = RfbPointerMapper::new();
    let size = FramebufferSize {
        width: 1920,
        height: 0,
    };

    assert_eq!(
        mapper.handle_pointer(&mut sink, 0, 0, 0, size),
        Err(RfbPointerError::Input(InputError::InvalidFramebufferSize {
            width: 1920,
            height: 0,
        }))
    );
    assert!(queue.accepted_batches().is_empty());
}

#[test]
fn wheel_release_does_not_generate_another_step() {
    let queue = FakeCommandQueue::new();
    let mut sink = Ch9329InputSink::new(queue.clone(), 0, MouseMode::Absolute);
    let mut mapper = RfbPointerMapper::new();

    mapper
        .handle_pointer(&mut sink, 0x08, 100, 200, full_hd())
        .unwrap();
    mapper
        .handle_pointer(&mut sink, 0x00, 100, 200, full_hd())
        .unwrap();

    let batches = queue.accepted_batches();
    assert_eq!(batches.len(), 2);
    assert_eq!(batches[0].frames().len(), 2);
    assert_eq!(batches[0].frames()[1].data()[6], 1);
    assert_eq!(batches[1].frames().len(), 1);
    assert_eq!(batches[1].frames()[0].data()[6], 0);
}
