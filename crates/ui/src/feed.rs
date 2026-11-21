//! The feed: short items pushed by other machines (an open window, Claude
//! Code waiting for input, a failed build), shown on the home view.

use crate::model::ItemColor;

pub const MAX_ITEMS: usize = 16;
pub const ID_MAX: usize = 32;
pub const TEXT_MAX: usize = 32;
pub const DETAIL_MAX: usize = 12;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub id: String,
    pub text: String,
    /// Shown instead of the elapsed time when not empty.
    pub detail: String,
    pub color: ItemColor,
    /// Monotonic time the item's current state began. Negative when the
    /// state began before the device booted.
    pub since_ms: i64,
    pub until_ms: Option<u64>,
}

impl Item {
    pub fn age_ms(&self, now_ms: u64) -> u64 {
        (now_ms as i64).saturating_sub(self.since_ms).max(0) as u64
    }
}

/// Items in display order: most urgent colour first, then the oldest.
#[derive(Default)]
pub struct Feed {
    items: Vec<Item>,
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= ID_MAX
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

/// Keeps printable characters only and cuts to `max` characters.
pub fn clean(s: &str, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect()
}

impl Feed {
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    pub fn get(&self, id: &str) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    /// Adds or replaces an item. An update that keeps text and colour keeps
    /// the item's age, so repeated reports of the same state (e.g. "window
    /// open" every minute) do not reset the time shown.
    pub fn upsert(&mut self, mut item: Item, explicit_since: bool) -> Result<(), String> {
        match self.items.iter().position(|i| i.id == item.id) {
            Some(pos) => {
                let old = &self.items[pos];
                if !explicit_since && old.text == item.text && old.color == item.color {
                    item.since_ms = old.since_ms;
                }
                self.items.remove(pos);
            }
            None if self.items.len() >= MAX_ITEMS => {
                return Err(format!("the feed holds at most {MAX_ITEMS} items"));
            }
            None => {}
        }
        let at = self.items.partition_point(|i| {
            (i.color, i.since_ms, &i.id) <= (item.color, item.since_ms, &item.id)
        });
        self.items.insert(at, item);
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.id != id);
        self.items.len() != before
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }

    pub fn expire(&mut self, now_ms: u64) {
        self.items
            .retain(|i| i.until_ms.is_none_or(|until| now_ms < until));
    }

    pub fn top(&self) -> Option<&Item> {
        self.items.first()
    }
}

/// Compact age: `now`, `12m`, `3h 05m`, `2d 4h`.
pub fn age(ms: u64) -> String {
    let m = ms / 60_000;
    match m {
        0 => "now".into(),
        1..=59 => format!("{m}m"),
        60..=1439 => format!("{}h {:02}m", m / 60, m % 60),
        _ => format!("{}d {}h", m / 1440, m % 1440 / 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, text: &str, color: ItemColor, since_ms: i64) -> Item {
        Item {
            id: id.into(),
            text: text.into(),
            detail: String::new(),
            color,
            since_ms,
            until_ms: None,
        }
    }

    fn ids(f: &Feed) -> Vec<&str> {
        f.items().iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn sorted_by_colour_then_age() {
        let mut f = Feed::default();
        f.upsert(item("a", "x", ItemColor::Green, 0), false)
            .unwrap();
        f.upsert(item("b", "x", ItemColor::Red, 5_000), false)
            .unwrap();
        f.upsert(item("c", "x", ItemColor::Red, 1_000), false)
            .unwrap();
        f.upsert(item("d", "x", ItemColor::Yellow, 0), false)
            .unwrap();
        assert_eq!(ids(&f), ["c", "b", "d", "a"]);
        assert_eq!(f.top().unwrap().id, "c");
    }

    #[test]
    fn same_state_keeps_its_age_and_a_new_state_resets_it() {
        let mut f = Feed::default();
        f.upsert(item("w", "Window open", ItemColor::Red, 1_000), false)
            .unwrap();
        f.upsert(item("w", "Window open", ItemColor::Red, 9_000), false)
            .unwrap();
        assert_eq!(f.get("w").unwrap().since_ms, 1_000);
        f.upsert(item("w", "Window open", ItemColor::Red, 9_000), true)
            .unwrap();
        assert_eq!(f.get("w").unwrap().since_ms, 9_000, "explicit age wins");
        f.upsert(item("w", "Window tilted", ItemColor::Yellow, 12_000), false)
            .unwrap();
        assert_eq!(f.get("w").unwrap().since_ms, 12_000);
        assert_eq!(f.items().len(), 1);
    }

    #[test]
    fn capacity_expiry_and_removal() {
        let mut f = Feed::default();
        for i in 0..MAX_ITEMS {
            f.upsert(item(&format!("i{i}"), "x", ItemColor::Blue, 0), false)
                .unwrap();
        }
        assert!(f
            .upsert(item("one-more", "x", ItemColor::Blue, 0), false)
            .is_err());
        assert!(
            f.upsert(item("i3", "y", ItemColor::Red, 0), false).is_ok(),
            "updating an existing item still works"
        );
        assert!(f.remove("i3"));
        assert!(!f.remove("i3"));

        f.clear();
        let mut short = item("t", "x", ItemColor::Blue, 0);
        short.until_ms = Some(5_000);
        f.upsert(short, false).unwrap();
        f.upsert(item("p", "x", ItemColor::Blue, 0), false).unwrap();
        f.expire(4_999);
        assert_eq!(f.items().len(), 2);
        f.expire(5_000);
        assert_eq!(ids(&f), ["p"]);
    }

    #[test]
    fn ids_and_text() {
        assert!(valid_id("claude"));
        assert!(valid_id("window.kitchen-1_a"));
        for bad in ["", "has space", "ümlaut", "a/b", &"x".repeat(ID_MAX + 1)] {
            assert!(!valid_id(bad), "{bad}");
        }
        assert_eq!(clean("a\nb\u{7}c", 2), "ab");
    }

    #[test]
    fn ages() {
        assert_eq!(age(59_999), "now");
        assert_eq!(age(60_000), "1m");
        assert_eq!(age(59 * 60_000), "59m");
        assert_eq!(age(65 * 60_000), "1h 05m");
        assert_eq!(age((26 * 60 + 30) * 60_000), "1d 2h");
    }
}
