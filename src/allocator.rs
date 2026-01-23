use core::alloc::{GlobalAlloc, Layout};
use core::ffi::c_void;
use flipperzero_sys::{free, malloc};

#[derive(Default)]
pub struct Allocator;

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        unsafe { malloc(layout.size() as u32) as *mut u8 }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, _layout: Layout) {
        unsafe {
            free(ptr as *mut c_void);
        }
    }
}

/// The static global allocator.
#[global_allocator]
static GLOBAL_ALLOCATOR: Allocator = Allocator;
