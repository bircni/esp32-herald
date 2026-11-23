//! What a person asks of the display, whatever the input: a touch gesture,
//! a button, the web app or an MQTT message.

use embedded_graphics::prelude::Point;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The next view or page (swipe left, rotary switch down).
    Next,
    /// The previous one (swipe right, rotary switch up).
    Previous,
    /// Back to the first view.
    Home,
    /// Opens the menu, where the layout has one.
    Menu,
    /// Redraws the whole display; on e-paper a full refresh that clears
    /// ghosting.
    Refresh,
    /// A tap at a point on the screen.
    Tap(Point),
    /// A long press at a point.
    LongPress(Point),
}
