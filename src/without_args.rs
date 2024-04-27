use std::{
    fmt::{self, Debug, Formatter},
    sync::atomic::{AtomicPtr, Ordering},
};

// internals
// ---------
//
// the inner atomic usize is a nullable pointer to a heap allocation.
// the pointed-to data consists of:
//
// - an `unsafe fn(bool, *mut u8)` which, when called with the pointer:
//
//   - if the bool is true, runs the callback (dropping it)
//   - if the bool is false, drops the callback without running it
//   - deallocates the heap allocation
// - padding
// - the `F: FnOnce() + Send + 'static` value

/// Like an `Atomic<Option<Box<dyn FnOnce() + Send + 'static>>>`.
///
/// See [`CallbackCellArgs`][crate::CallbackCellArgs] for a version with args.
pub struct CallbackCell(AtomicPtr<CallbackCellInner<()>>);

#[repr(C)]
struct CallbackCellInner<F> {
    fn_ptr: unsafe fn(bool, *mut CallbackCellInner<()>),
    tail: F,
}

impl CallbackCell {
    /// Construct with no callback.
    pub fn new() -> Self {
        CallbackCell(AtomicPtr::new(std::ptr::null_mut()))
    }

    /// Atomically set the callback.
    pub fn put<F: FnOnce() + Send + 'static>(&self, f: F) {
        let bx = Box::new(CallbackCellInner {
            fn_ptr: fn_ptr_impl::<F>,
            tail: f,
        });
        let ptr = Box::into_raw(bx);

        // atomic put
        let old_ptr = self.0.swap(ptr.cast(), Ordering::AcqRel);

        // clean up previous value
        unsafe {
            drop_raw(old_ptr);
        }
    }

    /// Atomically take the callback then run it.
    ///
    /// Returns true if a callback was present.
    pub fn take_call(&self) -> bool {
        // atomic take
        let ptr = self.0.swap(std::ptr::null_mut(), Ordering::AcqRel);

        // run it
        if !ptr.is_null() {
            unsafe {
                let fn_ptr = (*ptr).fn_ptr;
                fn_ptr(true, ptr);
            }
            true
        } else {
            false
        }
    }
}

impl Drop for CallbackCell {
    fn drop(&mut self) {
        unsafe {
            drop_raw(*self.0.get_mut());
        }
    }
}

// implementation for the function pointer for a given callback type F.
unsafe fn fn_ptr_impl<F: FnOnce() + Send + 'static>(run: bool, ptr: *mut CallbackCellInner<()>) {
    let ptr: *mut CallbackCellInner<F> = ptr.cast();
    let bx = unsafe { Box::from_raw(ptr) };

    // this part is basically safe code
    if run {
        (bx.tail)();
    }
}

// drop the pointed to data, including freeing the heap allocation, without running the callback,
// if the pointer is non-null.
unsafe fn drop_raw(ptr: *mut CallbackCellInner<()>) {
    if !ptr.is_null() {
        unsafe {
            let fn_ptr = (*ptr).fn_ptr;
            fn_ptr(false, ptr);
        }
    }
}

impl Default for CallbackCell {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for CallbackCell {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if (self.0.load(Ordering::Relaxed) as *const ()).is_null() {
            f.write_str("CallbackCell(NULL)")
        } else {
            f.write_str("CallbackCell(NOT NULL)")
        }
    }
}
