//! Hand-written `extern "C"` bindings for libevdi 1.14 (`/usr/include/evdi_lib.h`).
//! Only the subset used by the safe wrapper is exposed.

#![allow(non_camel_case_types, dead_code)]

use std::os::raw::{c_char, c_int, c_uint, c_void};

pub enum evdi_device_context {}
pub type evdi_handle = *mut evdi_device_context;
pub type evdi_selectable = c_int;

pub const AVAILABLE: c_int = 0;
pub const UNRECOGNIZED: c_int = 1;
pub const NOT_PRESENT: c_int = 2;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct evdi_rect {
    pub x1: c_int,
    pub y1: c_int,
    pub x2: c_int,
    pub y2: c_int,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct evdi_mode {
    pub width: c_int,
    pub height: c_int,
    pub refresh_rate: c_int,
    pub bits_per_pixel: c_int,
    pub pixel_format: c_uint,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct evdi_buffer {
    pub id: c_int,
    pub buffer: *mut c_void,
    pub width: c_int,
    pub height: c_int,
    pub stride: c_int,
    pub rects: *mut evdi_rect,
    pub rect_count: c_int,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct evdi_cursor_set {
    pub hot_x: i32,
    pub hot_y: i32,
    pub width: u32,
    pub height: u32,
    pub enabled: u8,
    pub buffer_length: u32,
    pub buffer: *mut u32,
    pub pixel_format: u32,
    pub stride: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct evdi_cursor_move {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct evdi_ddcci_data {
    pub address: u16,
    pub flags: u16,
    pub buffer_length: u32,
    pub buffer: *mut u8,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct evdi_event_context {
    pub dpms_handler: Option<extern "C" fn(dpms_mode: c_int, user_data: *mut c_void)>,
    pub mode_changed_handler: Option<extern "C" fn(mode: evdi_mode, user_data: *mut c_void)>,
    pub update_ready_handler:
        Option<extern "C" fn(buffer_to_be_updated: c_int, user_data: *mut c_void)>,
    pub crtc_state_handler: Option<extern "C" fn(state: c_int, user_data: *mut c_void)>,
    pub cursor_set_handler:
        Option<extern "C" fn(cursor_set: evdi_cursor_set, user_data: *mut c_void)>,
    pub cursor_move_handler:
        Option<extern "C" fn(cursor_move: evdi_cursor_move, user_data: *mut c_void)>,
    pub ddcci_data_handler:
        Option<extern "C" fn(ddcci_data: evdi_ddcci_data, user_data: *mut c_void)>,
    pub user_data: *mut c_void,
}

#[repr(C)]
pub struct evdi_lib_version {
    pub version_major: c_int,
    pub version_minor: c_int,
    pub version_patchlevel: c_int,
}

extern "C" {
    pub fn evdi_check_device(device: c_int) -> c_int;
    pub fn evdi_open(device: c_int) -> evdi_handle;
    pub fn evdi_add_device() -> c_int;
    pub fn evdi_close(handle: evdi_handle);
    pub fn evdi_connect(
        handle: evdi_handle,
        edid: *const u8,
        edid_length: c_uint,
        sku_area_limit: u32,
    );
    pub fn evdi_connect2(
        handle: evdi_handle,
        edid: *const u8,
        edid_length: c_uint,
        pixel_area_limit: u32,
        pixel_per_second_limit: u32,
    );
    pub fn evdi_disconnect(handle: evdi_handle);
    pub fn evdi_register_buffer(handle: evdi_handle, buffer: evdi_buffer);
    pub fn evdi_unregister_buffer(handle: evdi_handle, buffer_id: c_int);
    pub fn evdi_request_update(handle: evdi_handle, buffer_id: c_int) -> bool;
    pub fn evdi_grab_pixels(
        handle: evdi_handle,
        rects: *mut evdi_rect,
        num_rects: *mut c_int,
    );
    pub fn evdi_handle_events(handle: evdi_handle, evtctx: *mut evdi_event_context);
    pub fn evdi_get_event_ready(handle: evdi_handle) -> evdi_selectable;
    pub fn evdi_get_lib_version(version: *mut evdi_lib_version);
    #[allow(dead_code)]
    pub fn evdi_enable_cursor_events(handle: evdi_handle, enable: bool);
    #[allow(dead_code)]
    pub fn Xorg_running() -> bool;
    #[allow(dead_code)]
    pub fn evdi_open_attached_to_fixed(
        sysfs_parent_device: *const c_char,
        length: usize,
    ) -> evdi_handle;
    pub fn evdi_ddcci_response(
        handle: evdi_handle,
        buffer: *const u8,
        buffer_length: u32,
        result: bool,
    );
}
