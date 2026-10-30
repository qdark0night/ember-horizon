//! A mod as one built for another Ember ABI would be: it exports that ABI and,
//! like `ember_mod!` does, refuses a host that says another one.
#![allow(unsafe_code)]
use ember_api::abi::{Host, RawEvent, RawMod, Str};
use std::ffi::c_void;

const ABI: u32 = match option_env!("EMBER_PROBE_ABI") {
    Some(v) => parse(v.as_bytes()),
    None => ember_api::ABI_VERSION,
};

const fn parse(digits: &[u8]) -> u32 {
    let mut n = 0;
    let mut i = 0;
    while i < digits.len() {
        n = n * 10 + (digits[i] - b'0') as u32;
        i += 1;
    }
    n
}

#[unsafe(no_mangle)]
pub static EMBER_ABI: u32 = ABI;

unsafe extern "C" fn on_event(_: *mut c_void, _: *const Host, _: *mut RawEvent) -> u32 {
    0
}
unsafe extern "C" fn drop(_: *mut c_void) {}

static INSTANCE: u8 = 0;

/// # Safety
/// `host` is the game's host table.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ember_mod_entry(host: *const Host) -> RawMod {
    let abi = unsafe { (*host).abi };
    let ok = abi == ABI;
    RawMod {
        abi: ABI,
        id: Str::new("abi-probe"),
        name: Str::new("ABI probe"),
        version: Str::new("0.1.0"),
        instance: if ok {
            (&raw const INSTANCE).cast_mut().cast()
        } else {
            std::ptr::null_mut()
        },
        error: Str::new(if ok { "" } else { "the host says another ABI" }),
        on_event,
        drop,
    }
}
