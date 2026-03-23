use std::ptr::NonNull;
use std::sync::mpsc::Receiver;

use objc2::rc::Retained;
use objc2::Message;
use objc2_app_kit::{
    NSPanel, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectView, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSTimer};
use block2::RcBlock;

use tab_core::OverlayMessage;
use crate::view::{self, CandidateView};

const PANEL_WIDTH: f64 = 400.0;
const PANEL_ROW_HEIGHT: f64 = 28.0;
const MAX_VISIBLE_ROWS: usize = 8;

pub fn create_panel(mtm: MainThreadMarker) -> Retained<NSPanel> {
    let frame = NSRect::new(
        NSPoint::new(100.0, 100.0),
        NSSize::new(PANEL_WIDTH, PANEL_ROW_HEIGHT * MAX_VISIBLE_ROWS as f64),
    );

    let style = NSWindowStyleMask::Borderless
        | NSWindowStyleMask::NonactivatingPanel;

    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        mtm.alloc(),
        frame,
        style,
        objc2_app_kit::NSBackingStoreType(2), // NSBackingStoreBuffered = 2
        false,
    );

    // Floating window level + 1
    panel.setLevel(objc2_app_kit::NSFloatingWindowLevel + 1);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(
        &objc2_app_kit::NSColor::clearColor(),
    ));
    panel.setHasShadow(true);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Transient,
    );
    panel.setHidesOnDeactivate(false);

    panel
}

pub fn setup_panel(
    panel: &NSPanel,
    candidate_view: &CandidateView,
    mtm: MainThreadMarker,
) {
    let content_rect = panel.contentView().unwrap().frame();

    let effect_view = NSVisualEffectView::initWithFrame(
        mtm.alloc(),
        content_rect,
    );
    effect_view.setMaterial(NSVisualEffectMaterial::Popover);
    effect_view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect_view.setWantsLayer(true);

    effect_view.addSubview(candidate_view);
    panel.contentView().unwrap().addSubview(&effect_view);
    candidate_view.setFrame(content_rect);

    // Start hidden
    panel.orderOut(None);
}

pub fn start_message_poll(
    rx: Receiver<OverlayMessage>,
    panel: &NSPanel,
    candidate_view: &CandidateView,
    _mtm: MainThreadMarker,
) {
    let panel = panel.retain();
    let candidate_view = candidate_view.retain();

    let block = RcBlock::new(move |_timer: NonNull<NSTimer>| {
        while let Ok(msg) = rx.try_recv() {
            match msg {
                OverlayMessage::Show {
                    candidates,
                    selected,
                    ..
                } => {
                    if candidates.is_empty() {
                        panel.orderOut(None);
                        continue;
                    }

                    let row_count = candidates.len().min(MAX_VISIBLE_ROWS);
                    let height = PANEL_ROW_HEIGHT * row_count as f64;
                    let mut frame = panel.frame();
                    frame.size.height = height;
                    panel.setFrame_display(frame, true);

                    view::update_candidates(
                        &candidate_view,
                        &candidates,
                        selected as usize,
                    );
                    panel.orderFront(None);
                }

                OverlayMessage::Select { index, .. } => {
                    view::update_selection(&candidate_view, index as usize);
                }

                OverlayMessage::Hide { .. } => {
                    panel.orderOut(None);
                }
            }
        }
    });

    unsafe {
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            0.016,
            true,
            &block,
        );
    }
}

#[allow(dead_code)] // Used when accessibility positioning is wired
pub fn position_panel(
    panel: &NSPanel,
    screen_x: f64,
    screen_y: f64,
) {
    let frame = panel.frame();
    let origin = NSPoint::new(screen_x, screen_y - frame.size.height);
    panel.setFrameOrigin(origin);
}
