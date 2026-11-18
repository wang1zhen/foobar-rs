//! Minimal foobar-rs WASM plugin template.
//!
//! Core wasm, no WASI and no host imports. The host reads command names through
//! the fixed `fb_command_*` exports; names are static ASCII bytes, so index and
//! offset checks are just bounds checks.

const COMMAND_NAMES: [&[u8]; 2] = [b"Template Hello", b"Template World"];

fn command_name(index: i32) -> Option<&'static [u8]> {
    usize::try_from(index)
        .ok()
        .and_then(|index| COMMAND_NAMES.get(index))
        .copied()
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_command_count() -> i32 {
    COMMAND_NAMES.len() as i32
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_command_name_len(index: i32) -> i32 {
    command_name(index).map_or(-1, |name| name.len() as i32)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_command_name_byte(index: i32, offset: i32) -> i32 {
    command_name(index)
        .and_then(|name| {
            usize::try_from(offset)
                .ok()
                .and_then(|offset| name.get(offset))
                .copied()
        })
        .map_or(-1, i32::from)
}

#[unsafe(no_mangle)]
pub extern "C" fn fb_command_run(index: i32) -> i32 {
    index
}
