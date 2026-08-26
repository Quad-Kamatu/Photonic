use photonic_core::import_svg;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

struct TrackingAllocator;

static LIVE_ALLOCATED_BYTES: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            LIVE_ALLOCATED_BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE_ALLOCATED_BYTES.fetch_sub(layout.size(), Ordering::Relaxed);
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                LIVE_ALLOCATED_BYTES.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                LIVE_ALLOCATED_BYTES.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOCATOR: TrackingAllocator = TrackingAllocator;

fn live_allocated_bytes() -> usize {
    LIVE_ALLOCATED_BYTES.load(Ordering::Relaxed)
}

#[test]
fn repeated_styled_imports_do_not_retain_style_keys() {
    const SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
  <path FILL="#ff0000" d="M0 0 L10 0 L10 10 Z" style="fill:#00ff00;stroke:#0000ff;opacity:50%;fill-opacity:75%;stroke-opacity:80%;stroke-width:2;stroke-linecap:round;stroke-linejoin:round;stroke-miterlimit:4;stroke-dasharray:1,2;stroke-dashoffset:0;font-family:serif;font-size:12;font-weight:bold;text-anchor:middle;mix-blend-mode:normal"/>
</svg>"##;

    // Warm up one import so one-time allocations are included in the baseline.
    {
        let doc = import_svg(SVG).expect("SVG import should succeed");
        assert_eq!(doc.nodes.len(), 1);
    }
    let baseline = live_allocated_bytes();

    for _ in 0..128 {
        let doc = import_svg(SVG).expect("SVG import should succeed");
        std::hint::black_box(doc.nodes.len());
    }

    let growth = live_allocated_bytes().saturating_sub(baseline);
    assert!(
        growth <= 4096,
        "live allocations grew by {growth} bytes after dropped repeated imports"
    );
}
