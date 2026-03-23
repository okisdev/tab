use objc2::rc::Retained;
use objc2::{define_class, msg_send, AllocAnyThread, MainThreadOnly};
use objc2_app_kit::{NSAttributedStringNSStringDrawing, NSColor, NSFont, NSView};
use objc2_foundation::{
    ns_string, MainThreadMarker, NSMutableAttributedString,
    NSPoint, NSRect, NSSize, NSString,
};

use tab_core::Candidate;

const ROW_HEIGHT: f64 = 28.0;
const PADDING_LEFT: f64 = 12.0;
const PADDING_TOP: f64 = 4.0;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "TabCandidateView"]
    pub struct CandidateView;

    impl CandidateView {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty_rect: NSRect) {
            self.draw_candidates();
        }

        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

// Global state for candidate view (single instance, main thread only)
static mut CANDIDATES: Vec<Candidate> = Vec::new();
static mut SELECTED: usize = 0;

impl CandidateView {
    fn draw_candidates(&self) {
        let (candidates, selected) = unsafe { (&*(&raw const CANDIDATES), SELECTED) };

        let font = NSFont::monospacedSystemFontOfSize_weight(13.0, 0.0);
        let bold_font = NSFont::monospacedSystemFontOfSize_weight(13.0, 0.7);
        let frame = self.frame();

        for (i, candidate) in candidates.iter().enumerate() {
            let y = i as f64 * ROW_HEIGHT;
            if y > frame.size.height {
                break;
            }

            // Draw selection highlight
            if i == selected {
                let highlight_rect = NSRect::new(
                    NSPoint::new(0.0, y),
                    NSSize::new(frame.size.width, ROW_HEIGHT),
                );
                let color = NSColor::selectedContentBackgroundColor();
                color.setFill();
                let path =
                    objc2_app_kit::NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                        highlight_rect, 4.0, 4.0,
                    );
                path.fill();
            }

            // Draw command text with match highlighting
            let text = &candidate.text;
            let attr_string = unsafe {
                let s = NSMutableAttributedString::initWithString(
                    NSMutableAttributedString::alloc(),
                    &NSString::from_str(text),
                );

                let text_color = if i == selected {
                    NSColor::alternateSelectedControlTextColor()
                } else {
                    NSColor::labelColor()
                };

                let range = objc2_foundation::NSRange::new(0, text.len());
                let font_key = ns_string!("NSFont");
                let color_key = ns_string!("NSColor");
                s.addAttribute_value_range(font_key, &font, range);
                s.addAttribute_value_range(color_key, &text_color, range);

                // Highlight matched characters
                let highlight_color = if i == selected {
                    NSColor::alternateSelectedControlTextColor()
                } else {
                    NSColor::controlAccentColor()
                };

                for &pos in &candidate.match_positions {
                    let pos = pos as usize;
                    if pos < text.len() {
                        let match_range = objc2_foundation::NSRange::new(pos, 1);
                        s.addAttribute_value_range(font_key, &bold_font, match_range);
                        if i != selected {
                            s.addAttribute_value_range(
                                color_key,
                                &highlight_color,
                                match_range,
                            );
                        }
                    }
                }

                s
            };

            let draw_point = NSPoint::new(PADDING_LEFT, y + PADDING_TOP);
            attr_string.drawAtPoint(draw_point);
        }
    }
}

pub fn create_candidate_view(mtm: MainThreadMarker) -> Retained<CandidateView> {
    let frame = NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(400.0, ROW_HEIGHT * 8.0),
    );
    unsafe { msg_send![mtm.alloc::<CandidateView>(), initWithFrame: frame] }
}

pub fn update_candidates(view: &CandidateView, candidates: &[Candidate], selected: usize) {
    unsafe {
        CANDIDATES = candidates.to_vec();
        SELECTED = selected;
    }
    view.setNeedsDisplay(true);
}

pub fn update_selection(view: &CandidateView, selected: usize) {
    unsafe {
        SELECTED = selected;
    }
    view.setNeedsDisplay(true);
}
