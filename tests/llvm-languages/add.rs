#![no_std]

#[no_mangle]
pub extern "C" fn lume_test_add_rust(a: i32, b: i32) -> i32 {
    a.wrapping_add(b)
}
