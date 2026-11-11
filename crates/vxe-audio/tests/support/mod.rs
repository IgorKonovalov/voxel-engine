//! The counting allocator the allocation tests share: it counts the heap allocations and frees
//! a thread makes while [`count`] runs on it.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

/// Forwards to `System`, counting on threads inside [`count`].
pub struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    static FREES: Cell<u64> = const { Cell::new(0) };
}

fn bump(counter: &'static std::thread::LocalKey<Cell<u64>>) {
    if COUNTING.with(Cell::get) {
        counter.with(|c| c.set(c.get() + 1));
    }
}

// SAFETY: forwards every call to `System` unchanged; the thread-locals are `const`-initialised
// `Cell`s, which never allocate.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        bump(&ALLOCATIONS);
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, passed through.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        bump(&FREES);
        // SAFETY: `ptr` came from `System.alloc` with this layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    /// One allocation: a grown block, moved or not.
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        bump(&ALLOCATIONS);
        // SAFETY: as `alloc` and `dealloc`, passed through.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

/// Allocations and frees counted over one [`count`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub allocations: u64,
    pub frees: u64,
}

impl Counts {
    /// Nothing allocated and nothing freed.
    pub const NONE: Counts = Counts {
        allocations: 0,
        frees: 0,
    };
}

/// Runs `f` on this thread with counting on, and returns its value and what it allocated and
/// freed. Counts made on other threads are not seen.
pub fn count<R>(f: impl FnOnce() -> R) -> (R, Counts) {
    let read = || Counts {
        allocations: ALLOCATIONS.with(Cell::get),
        frees: FREES.with(Cell::get),
    };
    let before = read();
    COUNTING.with(|c| c.set(true));
    let r = f();
    COUNTING.with(|c| c.set(false));
    let after = read();
    (
        r,
        Counts {
            allocations: after.allocations - before.allocations,
            frees: after.frees - before.frees,
        },
    )
}
