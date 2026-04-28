//! Custom menubar icon, drawn at runtime via NSBezierPath.
//!
//! Marked as a template image so AppKit auto-tints it for light/dark
//! menubars (we draw in solid black + alpha; AppKit handles the rest).

#![allow(deprecated)] // lockFocus is the simplest path; it still works.

use objc2::rc::Retained;
use objc2::AllocAnyThread;
use objc2_app_kit::{NSBezierPath, NSColor, NSImage};
use objc2_core_foundation::CGFloat;
use objc2_foundation::{NSPoint, NSRect, NSSize};

const ICON_SIZE: CGFloat = 18.0;

pub fn build_template_icon() -> Retained<NSImage> {
    unsafe {
        let size = NSSize::new(ICON_SIZE, ICON_SIZE);
        let image = NSImage::initWithSize(NSImage::alloc(), size);

        image.lockFocus();
        draw_icon();
        image.unlockFocus();

        image.setTemplate(true);
        image
    }
}

unsafe fn draw_icon() {
    NSColor::blackColor().setFill();
    NSColor::blackColor().setStroke();

    // Microphone capsule (rounded rect): centered, slightly left of center
    // to leave room for the "hush" wave marks on the right.
    let capsule_w: CGFloat = 5.0;
    let capsule_h: CGFloat = 8.0;
    let cap_x = (ICON_SIZE - capsule_w) / 2.0 - 2.5;
    let cap_y = (ICON_SIZE - capsule_h) / 2.0 + 2.5;
    let capsule_rect = NSRect::new(NSPoint::new(cap_x, cap_y), NSSize::new(capsule_w, capsule_h));
    let capsule = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
        capsule_rect,
        capsule_w / 2.0,
        capsule_w / 2.0,
    );
    capsule.fill();

    // Stand: horseshoe arc under the capsule.
    let arc = NSBezierPath::bezierPath();
    let cx = cap_x + capsule_w / 2.0;
    let arc_top_y = cap_y;
    let arc_radius: CGFloat = 4.5;
    arc.setLineWidth(1.4);
    arc.moveToPoint(NSPoint::new(cx - arc_radius, arc_top_y));
    arc.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle_clockwise(
        NSPoint::new(cx, arc_top_y),
        arc_radius,
        180.0,
        360.0,
        false,
    );
    arc.stroke();

    // Stem + base.
    let stem_y_top = arc_top_y - arc_radius;
    let stem = NSBezierPath::bezierPath();
    stem.setLineWidth(1.4);
    stem.moveToPoint(NSPoint::new(cx, stem_y_top));
    stem.lineToPoint(NSPoint::new(cx, stem_y_top - 2.0));
    stem.stroke();

    let base = NSBezierPath::bezierPath();
    base.setLineWidth(1.4);
    base.moveToPoint(NSPoint::new(cx - 2.0, stem_y_top - 2.0));
    base.lineToPoint(NSPoint::new(cx + 2.0, stem_y_top - 2.0));
    base.stroke();

    // "hush" sound waves: two short vertical ticks suggesting muted /
    // quiet speech rather than blaring sound.
    let wave_x = cap_x + capsule_w + 1.5;
    let wave_cy = cap_y + capsule_h / 2.0;
    for (i, len) in [2.0_f64, 3.0].iter().enumerate() {
        let dx = wave_x + (i as CGFloat) * 1.8;
        let half = (*len as CGFloat) / 2.0;
        let w = NSBezierPath::bezierPath();
        w.setLineWidth(1.2);
        w.moveToPoint(NSPoint::new(dx, wave_cy - half));
        w.lineToPoint(NSPoint::new(dx, wave_cy + half));
        w.stroke();
    }
}
