use std::any::type_name;

use ipkvm_core::{InputResult, InputSink, KeyEvent, MouseMode, PointerEvent};
use ipkvm_headless::rfb_input::{
    RfbControllerReleaseReason, RfbInputError, RfbInputEventError, RfbInputEventKind,
    RfbInputLifecycleError, RfbInputNotice, RfbInputOperation, RfbInputPump, RfbInputRunError,
    RfbKeyboardRejection,
};

struct NoopSink;

impl InputSink for NoopSink {
    fn set_mouse_mode(&mut self, _mode: MouseMode) -> InputResult<()> {
        Ok(())
    }

    fn handle_key_batch(&mut self, _events: &[KeyEvent]) -> InputResult<()> {
        Ok(())
    }

    fn handle_pointer_batch(&mut self, _events: &[PointerEvent]) -> InputResult<()> {
        Ok(())
    }

    fn release_all(&mut self) -> InputResult<()> {
        Ok(())
    }
}

#[test]
fn public_input_pump_contract_types_are_available() {
    let names = [
        type_name::<RfbControllerReleaseReason>(),
        type_name::<RfbInputError>(),
        type_name::<RfbInputEventError>(),
        type_name::<RfbInputEventKind>(),
        type_name::<RfbInputLifecycleError>(),
        type_name::<RfbInputNotice>(),
        type_name::<RfbInputOperation>(),
        type_name::<RfbInputPump<NoopSink>>(),
        type_name::<RfbInputRunError>(),
        type_name::<RfbKeyboardRejection>(),
    ];

    assert!(names.iter().all(|name| name.starts_with("ipkvm_headless")));
}
