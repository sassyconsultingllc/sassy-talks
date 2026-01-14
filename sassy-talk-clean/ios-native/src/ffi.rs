/// FFI Module - C bindings for Swift
/// 
/// Provides C-compatible interface for Swift to call Rust functions

pub mod exports {
    // Re-export all FFI functions from lib.rs
    // This module organizes the FFI interface
    
    // The actual FFI functions are defined in lib.rs with #[no_mangle]
    // This provides documentation and organization
}

/// Helper functions for FFI conversions
pub mod helpers {
    use std::ffi::{CStr, CString};
    use std::os::raw::c_char;
    
    /// Convert Rust String to C string
    /// 
    /// # Safety
    /// Caller must free with sassytalkie_free_string
    pub unsafe fn rust_string_to_c(s: String) -> *const c_char {
        CString::new(s).unwrap().into_raw()
    }
    
    /// Convert C string to Rust String
    /// 
    /// # Safety
    /// Pointer must be valid C string
    pub unsafe fn c_string_to_rust(s: *const c_char) -> Option<String> {
        if s.is_null() {
            return None;
        }
        CStr::from_ptr(s).to_str().ok().map(|s| s.to_string())
    }
}
