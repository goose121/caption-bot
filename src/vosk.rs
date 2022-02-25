pub mod sys {
    use std::os::raw::{c_char, c_int, c_short};
    use std::marker::PhantomData;

    #[repr(C)]
    pub struct VoskModel {
        _data: [u8; 0]
    }
    #[repr(C)]
    pub struct VoskRecognizer {
        _data: [u8; 0]
    }

    #[link(name = "vosk")]
    extern "C" {
        pub fn vosk_model_new(path: *const c_char) -> *mut VoskModel;
        pub fn vosk_recognizer_new(model: *mut VoskModel, sample_rate: f32) -> *mut VoskRecognizer;

        pub fn vosk_model_find_word(model: *mut VoskModel, word: *const c_char) -> c_int;
        
        pub fn vosk_recognizer_accept_waveform(rec: *mut VoskRecognizer, buf: *const u8, len: usize) -> c_int;
        pub fn vosk_recognizer_accept_waveform_s(rec: *mut VoskRecognizer, buf: *const c_short, len: usize) -> c_int;
        pub fn vosk_recognizer_result(rec: *mut VoskRecognizer) -> *mut c_char;
        pub fn vosk_recognizer_partial_result(rec: *mut VoskRecognizer) -> *mut c_char;
        pub fn vosk_recognizer_final_result(rec: *mut VoskRecognizer) -> *mut c_char;
        pub fn vosk_recognizer_set_max_alternatives(rec: *mut VoskRecognizer, n: c_int);
        pub fn vosk_recognizer_reset(rec: *mut VoskRecognizer);
        
        pub fn vosk_recognizer_free(rec: *mut VoskRecognizer);
        pub fn vosk_model_free(model: *mut VoskModel);
    }
}

use std::path::Path;
use std::ffi::{CString, CStr};
use std::os::raw::{c_int, c_uint};
use serde::Deserialize;

pub struct Model(*mut sys::VoskModel);

unsafe impl Send for Model {}
unsafe impl Sync for Model {}

impl Model {
    pub fn new(path: impl AsRef<Path>) -> Model {
        use std::os::unix::ffi::OsStrExt;
        let path = CString::new(path.as_ref().to_owned().as_os_str().as_bytes()).unwrap();

        unsafe {
            Model(sys::vosk_model_new(path.as_ptr()))
        }
    }

    pub fn find_word(&mut self, word: &str) -> Option<c_uint> {
        let word = CString::new(word).unwrap();

        let res = unsafe {
            sys::vosk_model_find_word(self.0, word.as_ptr())
        };

        if res == -1 {
            None
        } else {
            Some(res as c_uint)
        }
    }
}

impl Drop for Model {
    fn drop(&mut self) {
        unsafe {
            sys::vosk_model_free(self.0);
        }
    }
}

pub struct Recognizer(*mut sys::VoskRecognizer);

unsafe impl Send for Recognizer {}
unsafe impl Sync for Recognizer {}

impl Recognizer {
    pub fn new(model: &Model, sample_rate: f32) -> Recognizer {
        unsafe {
            Recognizer(sys::vosk_recognizer_new(model.0, sample_rate))
        }
    }

    pub fn accept_waveform(&mut self, data: &[u8]) -> bool {
        let res = unsafe {
            sys::vosk_recognizer_accept_waveform(self.0, data.as_ptr(), data.len())
        };

        res == 1
    }

    pub fn accept_waveform_i16(&mut self, data: &[i16]) -> bool {
        let res = unsafe {
            sys::vosk_recognizer_accept_waveform_s(self.0, data.as_ptr(), data.len())
        };

        res == 1
    }

    // Result methods must take `self` mutably like this because
    // the result buffer is stored in the `VoxRecognizer` itself
    // and set with every result call

    pub fn result_json(&mut self) -> &CStr {
        unsafe {
            let s = sys::vosk_recognizer_result(self.0);

            CStr::from_ptr(s)
        }
    }

    pub fn partial_result_json(&mut self) -> &CStr {
        unsafe {
            let s = sys::vosk_recognizer_partial_result(self.0);

            CStr::from_ptr(s)
        }
    }

    pub fn final_result_json(&mut self) -> &CStr {
        unsafe {
            let s = sys::vosk_recognizer_final_result(self.0);

            CStr::from_ptr(s)
        }
    }

    pub fn set_max_alternatives(&mut self, n: c_int) {
        unsafe {
            sys::vosk_recognizer_set_max_alternatives(self.0, n);
        }
    }

    pub fn reset(&mut self) {
        unsafe {
            sys::vosk_recognizer_reset(self.0);
        }
    }
}

#[derive(Deserialize)]
pub struct SimpleResult {
    pub text: String
}

impl Drop for Recognizer {
    fn drop(&mut self) {
        unsafe {
            sys::vosk_recognizer_free(self.0);
        }
    }
}
