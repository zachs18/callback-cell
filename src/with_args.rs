use std::{
    fmt::{self, Debug, Formatter},
    marker::PhantomData,
    mem::ManuallyDrop,
    sync::atomic::{AtomicPtr, Ordering},
};

// internals
// ---------
//
// the inner atomic usize is a nullable pointer to a heap allocation.
// the pointed-to data consists of:
//
// - an `unsafe fn(Option<&mut union { I, O }, *mut u8)` which, when
//   called with the pointer:
//
//   - if the option is Some, reads the input from the union, runs the
//     callback with the input (dropping it and the input), and writes
//     the output back to the union
//   - if the option is None, drops the callback without running it
//   - deallocates the heap allocation
// - padding
// - the `F: FnOnce() + Send + 'static` value

/// Like an `Atomic<Option<Box<dyn FnOnce(I) -> O + Send + 'static>>>`.
///
/// It's a normal [`CallbackCell`][crate::CallbackCell] but with args.
pub struct CallbackCellArgs<I, O> {
    ptr: AtomicPtr<CallbackCellInner<(), I, O>>,
    _p: PhantomData<dyn FnOnce(I) -> O + Send + 'static>,
}

#[repr(C)]
struct CallbackCellInner<F, I, O> {
    fn_ptr: unsafe fn(Option<&mut IoSlot<I, O>>, *mut CallbackCellInner<(), I, O>),
    tail: F,
}

impl<I, O> CallbackCellArgs<I, O> {
    /// Construct with no callback.
    pub fn new() -> Self {
        CallbackCellArgs {
            ptr: AtomicPtr::new(std::ptr::null_mut()),
            _p: PhantomData,
        }
    }

    /// Atomically set the callback.
    ///
    /// Makes only one heap allocation. Any callback previously present is dropped.
    pub fn put<F: FnOnce(I) -> O + Send + 'static>(&self, f: F) {
        let bx = Box::new(CallbackCellInner {
            fn_ptr: fn_ptr_impl::<F, I, O>,
            tail: f,
        });
        let ptr = Box::into_raw(bx);

        // atomic put
        let old_ptr = self.ptr.swap(ptr.cast(), Ordering::AcqRel);

        // clean up previous value
        unsafe {
            drop_raw(old_ptr);
        }
    }

    /// Atomically take the callback then run it with the given input.
    ///
    /// Returns the output if a callback was present. If a callback was not
    /// present, returns the original input.
    pub fn take_call(&self, input: I) -> Result<O, I> {
        // atomic take
        let ptr = self.ptr.swap(std::ptr::null_mut(), Ordering::AcqRel);
        // run it
        if !ptr.is_null() {
            let fn_ptr = unsafe { (*ptr).fn_ptr };
            let mut io_slot = IoSlot {
                input: ManuallyDrop::new(input),
            };
            unsafe { fn_ptr(Some(&mut io_slot), ptr) };
            Ok(ManuallyDrop::into_inner(unsafe { io_slot.output }))
        } else {
            Err(input)
        }
    }
}

impl<I, O> Drop for CallbackCellArgs<I, O> {
    fn drop(&mut self) {
        unsafe {
            drop_raw(*self.ptr.get_mut());
        }
    }
}

union IoSlot<I, O> {
    input: ManuallyDrop<I>,
    output: ManuallyDrop<O>,
}

// implementation for the function pointer for a given callback type F.
unsafe fn fn_ptr_impl<F, I, O>(
    run: Option<&mut IoSlot<I, O>>,
    ptr: *mut CallbackCellInner<(), I, O>,
) where
    F: FnOnce(I) -> O + Send + 'static,
{
    let ptr: *mut CallbackCellInner<F, I, O> = ptr.cast();
    let bx = unsafe { Box::from_raw(ptr) };

    // this part is basically safe code
    if let Some(io) = run {
        let input = unsafe { ManuallyDrop::take(&mut io.input) };
        let output = (bx.tail)(input);
        io.output = ManuallyDrop::new(output);
    }
}

// drop the pointed to data, including freeing the heap allocation, without running the callback,
// if the pointer is non-null.
unsafe fn drop_raw<I, O>(ptr: *mut CallbackCellInner<(), I, O>) {
    if !ptr.is_null() {
        unsafe {
            let fn_ptr = (*ptr).fn_ptr;
            fn_ptr(None, ptr);
        }
    }
}

impl<I, O> Default for CallbackCellArgs<I, O> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I, O> Debug for CallbackCellArgs<I, O> {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        if (self.ptr.load(Ordering::Relaxed) as *const ()).is_null() {
            f.write_str("CallbackCellArgs(NULL)")
        } else {
            f.write_str("CallbackCellArgs(NOT NULL)")
        }
    }
}
