//! HID decisions for the Windows Precision Touchpad backend that need no
//! Win32: which link collections hold fingers, which range to normalise
//! against, how to read a raw value, when a report means "touching", where
//! that puts the pad, and when a touch has gone stale.
//!
//! Everything here works on plain numbers copied out of the HidP structures,
//! so it builds and is unit-tested on every platform; the Win32 calls live in
//! `windows.rs`.

use std::time::Duration;

/// HID usage page: Generic Desktop.
pub(crate) const PAGE_GENERIC_DESKTOP: u16 = 0x01;
/// HID usage page: Digitizer.
pub(crate) const PAGE_DIGITIZER: u16 = 0x0D;
/// Digitizer usage of a Precision Touchpad top-level collection.
pub(crate) const USAGE_TOUCH_PAD: u16 = 0x05;
/// Generic Desktop X.
pub(crate) const USAGE_X: u16 = 0x30;
/// Generic Desktop Y.
pub(crate) const USAGE_Y: u16 = 0x31;
/// Digitizer Tip Switch: the contact is on the surface.
pub(crate) const USAGE_TIP_SWITCH: u16 = 0x42;
/// Digitizer Contact Count: contacts in the current frame.
pub(crate) const USAGE_CONTACT_COUNT: u16 = 0x54;
/// Upper bound on finger collections per report and contacts per frame.
/// Precision Touchpads report at most 5 contacts; the headroom keeps a
/// garbage contact count from stalling the frame logic for long.
pub(crate) const MAX_CONTACTS: usize = 10;

/// True for the HID top-level collection of a Windows Precision Touchpad.
pub(crate) fn is_precision_touchpad(usage_page: u16, usage: u16) -> bool {
    usage_page == PAGE_DIGITIZER && usage == USAGE_TOUCH_PAD
}

/// Usages covered by one `HIDP_VALUE_CAPS`: `NotRange.Usage` alone when
/// `IsRange` is 0, otherwise `Range.UsageMin..=Range.UsageMax` (ordered, in
/// case a descriptor lists them backwards).
pub(crate) fn usage_span(is_range: bool, usage: u16, usage_min: u16, usage_max: u16) -> (u16, u16) {
    if is_range {
        (usage_min.min(usage_max), usage_min.max(usage_max))
    } else {
        (usage, usage)
    }
}

/// One input value capability, copied out of `HIDP_VALUE_CAPS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ValueCap {
    pub usage_page: u16,
    /// First usage covered (see [`usage_span`]).
    pub usage_min: u16,
    /// Last usage covered (equal to `usage_min` when not a range).
    pub usage_max: u16,
    pub link_collection: u16,
    pub bit_size: u16,
    pub logical_min: i32,
    pub logical_max: i32,
    pub physical_min: i32,
    pub physical_max: i32,
}

impl ValueCap {
    pub(crate) fn covers(&self, usage_page: u16, usage: u16) -> bool {
        self.usage_page == usage_page && (self.usage_min..=self.usage_max).contains(&usage)
    }
}

/// The range a value is normalised against: the logical range, or the
/// physical one when the logical range is degenerate (`max <= min`).
pub(crate) fn axis_range(
    logical_min: i32,
    logical_max: i32,
    physical_min: i32,
    physical_max: i32,
) -> (i32, i32) {
    if logical_max > logical_min {
        (logical_min, logical_max)
    } else {
        (physical_min, physical_max)
    }
}

/// How to turn one axis' raw report value into a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Axis {
    /// Range for `normalize_axis` (see [`axis_range`]).
    pub min: i32,
    pub max: i32,
    bit_size: u16,
    /// The field is two's complement (HID: negative logical minimum).
    signed: bool,
}

impl Axis {
    pub(crate) fn from_cap(cap: &ValueCap) -> Self {
        let (min, max) = axis_range(
            cap.logical_min,
            cap.logical_max,
            cap.physical_min,
            cap.physical_max,
        );
        Self {
            min,
            max,
            bit_size: cap.bit_size,
            signed: cap.logical_min < 0,
        }
    }

    /// `HidP_GetUsageValue` returns the field's bits without sign
    /// extension; widen them to the logical value.
    pub(crate) fn decode(&self, raw: u32) -> i32 {
        match (self.signed, self.bit_size) {
            (true, bits @ 1..=31) => {
                let shift = 32 - u32::from(bits);
                ((raw << shift) as i32) >> shift
            }
            // 32 bits (or a nonsense size): plain two's complement.
            (true, _) => raw as i32,
            (false, _) => i32::try_from(raw).unwrap_or(i32::MAX),
        }
    }
}

/// One finger collection: where its X/Y live and how to read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Finger {
    pub link_collection: u16,
    pub x: Axis,
    pub y: Axis,
}

/// What a touchpad's reports look like.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    /// Finger collections in ascending link-collection order; `fingers[0]`
    /// is the first finger collection. Never empty.
    pub fingers: Vec<Finger>,
    /// The device reports Contact Count (Digitizer 0x54).
    pub has_contact_count: bool,
}

/// Finger collections are the non-zero link collections carrying both
/// Generic Desktop X and Y; the lowest one is the first finger. `None` when
/// there is none (not a usable touchpad).
pub(crate) fn layout(caps: &[ValueCap]) -> Option<Layout> {
    let find = |link: u16, usage: u16| {
        caps.iter()
            .find(|c| c.link_collection == link && c.covers(PAGE_GENERIC_DESKTOP, usage))
    };
    let mut links: Vec<u16> = caps
        .iter()
        .filter(|c| c.link_collection != 0 && c.covers(PAGE_GENERIC_DESKTOP, USAGE_X))
        .map(|c| c.link_collection)
        .collect();
    links.sort_unstable();
    links.dedup();
    let fingers: Vec<Finger> = links
        .into_iter()
        .filter_map(|link| {
            Some(Finger {
                link_collection: link,
                x: Axis::from_cap(find(link, USAGE_X)?),
                y: Axis::from_cap(find(link, USAGE_Y)?),
            })
        })
        .take(MAX_CONTACTS)
        .collect();
    if fingers.is_empty() {
        return None;
    }
    let has_contact_count = caps
        .iter()
        .any(|c| c.covers(PAGE_DIGITIZER, USAGE_CONTACT_COUNT));
    Some(Layout {
        fingers,
        has_contact_count,
    })
}

/// Tip switch state of a finger collection, from the Digitizer usages that
/// `HidP_GetUsages` reports as on.
pub(crate) fn tip_on(usages_on: &[u16]) -> bool {
    usages_on.contains(&USAGE_TIP_SWITCH)
}

/// One finger collection of one report, decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Contact {
    pub tip: bool,
    pub x: i32,
    pub y: i32,
}

/// What a report means for the pad state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Update {
    /// Leave the state as it is.
    Keep,
    /// The first touching contact of the frame, read from finger collection
    /// `finger` (index into [`Layout::fingers`]).
    Touch { finger: usize, x: i32, y: i32 },
    /// Nothing is touching.
    Lift,
}

/// Groups reports into frames and picks the contact that drives the pad.
///
/// Parallel-mode touchpads send every contact of a frame in one report.
/// Hybrid-mode touchpads have fewer finger collections than contacts and
/// spread a frame over several reports: the first carries the real Contact
/// Count, the rest carry 0. A 0 is therefore only "no touch" when no frame
/// is still waiting for contacts. Contacts past the count are padding and
/// are never read.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameTracker {
    /// Contacts announced for the current frame and not yet seen.
    remaining: usize,
    /// The current frame already produced a touching contact.
    found: bool,
}

impl FrameTracker {
    /// Feed one report. `contact_count` is `None` when the device has no
    /// Contact Count (every report is then a whole frame); `fingers` is the
    /// number of finger collections per report; `read(i)` decodes finger
    /// collection `i`, or `None` when it cannot be read (counts as lifted).
    pub(crate) fn feed(
        &mut self,
        contact_count: Option<u32>,
        fingers: usize,
        mut read: impl FnMut(usize) -> Option<Contact>,
    ) -> Update {
        let fingers = fingers.min(MAX_CONTACTS);
        if fingers == 0 {
            *self = Self::default();
            return Update::Keep;
        }
        match contact_count {
            Some(0) if self.remaining == 0 => {
                self.found = false;
                return Update::Lift;
            }
            // Hybrid continuation: more contacts of the current frame.
            Some(0) => {}
            Some(n) => {
                self.remaining = usize::try_from(n).unwrap_or(MAX_CONTACTS).min(MAX_CONTACTS);
                self.found = false;
            }
            None => {
                self.remaining = fingers;
                self.found = false;
            }
        }
        let mut update = Update::Keep;
        for finger in 0..fingers {
            if self.remaining == 0 {
                break;
            }
            self.remaining -= 1;
            if let Some(c) = read(finger) {
                if c.tip && !self.found {
                    self.found = true;
                    update = Update::Touch {
                        finger,
                        x: c.x,
                        y: c.y,
                    };
                }
            }
        }
        if self.remaining == 0 && !self.found {
            update = Update::Lift;
        }
        update
    }
}

/// What one report does to the published pad state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum PadChange {
    /// Leave the state as it is.
    Keep,
    /// A finger is down here: normalised, 0..=1, `y` grows upwards.
    Touch { x: f32, y: f32 },
    /// Nothing is touching; the last position stays.
    Lift,
}

/// Turn a frame decision into a pad position. HID `y` grows downwards and
/// the pad's grows upwards, so `y` is flipped. A finger index outside the
/// layout (never produced by [`FrameTracker`]) changes nothing.
pub(crate) fn pad_change(layout: &Layout, update: Update) -> PadChange {
    match update {
        Update::Keep => PadChange::Keep,
        Update::Lift => PadChange::Lift,
        Update::Touch { finger, x, y } => match layout.fingers.get(finger) {
            Some(f) => PadChange::Touch {
                x: crate::unit(crate::normalize_axis(x, f.x.min, f.x.max)),
                y: crate::unit(1.0 - crate::normalize_axis(y, f.y.min, f.y.max)),
            },
            None => PadChange::Keep,
        },
    }
}

/// A touch with no touch report for this long is stale. Precision
/// Touchpads report at 125 Hz or more while a contact is down, so this is
/// well over a dozen missed reports.
pub(crate) const STALE_TOUCH: Duration = Duration::from_millis(150);

/// Watchdog: the pad says `touching` but no touch report arrived for
/// `since_last_report` (`None`: never). The lift was lost (dropped report,
/// input taken by another process, session locked), so the pad must lift.
pub(crate) fn touch_is_stale(touching: bool, since_last_report: Option<Duration>) -> bool {
    touching && since_last_report.is_none_or(|d| d >= STALE_TOUCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A single-usage value cap.
    fn cap(
        page: u16,
        usage: u16,
        link: u16,
        logical: (i32, i32),
        physical: (i32, i32),
    ) -> ValueCap {
        ValueCap {
            usage_page: page,
            usage_min: usage,
            usage_max: usage,
            link_collection: link,
            bit_size: 16,
            logical_min: logical.0,
            logical_max: logical.1,
            physical_min: physical.0,
            physical_max: physical.1,
        }
    }

    fn xy(link: u16, x_max: i32, y_max: i32) -> [ValueCap; 2] {
        [
            cap(PAGE_GENERIC_DESKTOP, USAGE_X, link, (0, x_max), (0, 1200)),
            cap(PAGE_GENERIC_DESKTOP, USAGE_Y, link, (0, y_max), (0, 800)),
        ]
    }

    /// Typical 5-finger parallel-mode Precision Touchpad, caps out of order,
    /// with Contact Count, Scan Time and per-finger Contact ID.
    fn ptp_caps() -> Vec<ValueCap> {
        let mut caps = vec![
            cap(PAGE_DIGITIZER, USAGE_CONTACT_COUNT, 0, (0, 5), (0, 0)),
            cap(PAGE_DIGITIZER, 0x56, 0, (0, 65535), (0, 0)), // scan time
        ];
        for link in [3u16, 1, 5, 2, 4] {
            caps.push(cap(PAGE_DIGITIZER, 0x51, link, (0, 15), (0, 0))); // contact id
            caps.extend(xy(link, 1216 + i32::from(link), 800));
        }
        caps
    }

    fn touch(x: i32, y: i32) -> Option<Contact> {
        Some(Contact { tip: true, x, y })
    }

    fn lifted(x: i32, y: i32) -> Option<Contact> {
        Some(Contact { tip: false, x, y })
    }

    fn moved(finger: usize, x: i32, y: i32) -> Update {
        Update::Touch { finger, x, y }
    }

    /// Feed a report whose finger collections hold `contacts`; collections
    /// past the slice read as `None`. Also returns which indices were read.
    fn feed(
        t: &mut FrameTracker,
        count: Option<u32>,
        fingers: usize,
        contacts: &[Option<Contact>],
    ) -> (Update, Vec<usize>) {
        let mut read = Vec::new();
        let u = t.feed(count, fingers, |i| {
            read.push(i);
            contacts.get(i).copied().flatten()
        });
        (u, read)
    }

    #[test]
    fn precision_touchpad_is_digitizer_touch_pad_only() {
        assert!(is_precision_touchpad(0x0D, 0x05));
        assert!(!is_precision_touchpad(0x0D, 0x04)); // touch screen
        assert!(!is_precision_touchpad(0x01, 0x02)); // mouse
        assert!(!is_precision_touchpad(0x01, 0x05));
    }

    #[test]
    fn usage_span_uses_not_range_usage_unless_is_range() {
        assert_eq!(usage_span(false, 0x30, 0x10, 0x20), (0x30, 0x30));
        assert_eq!(usage_span(true, 0x99, 0x30, 0x31), (0x30, 0x31));
        assert_eq!(usage_span(true, 0x99, 0x31, 0x30), (0x30, 0x31));
    }

    #[test]
    fn covers_checks_page_and_range() {
        let mut c = cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, 100), (0, 0));
        assert!(c.covers(PAGE_GENERIC_DESKTOP, USAGE_X));
        assert!(!c.covers(PAGE_GENERIC_DESKTOP, USAGE_Y));
        assert!(!c.covers(PAGE_DIGITIZER, USAGE_X));
        c.usage_max = USAGE_Y;
        assert!(c.covers(PAGE_GENERIC_DESKTOP, USAGE_X));
        assert!(c.covers(PAGE_GENERIC_DESKTOP, USAGE_Y));
        assert!(!c.covers(PAGE_GENERIC_DESKTOP, 0x32));
    }

    #[test]
    fn axis_range_prefers_logical_and_falls_back_to_physical() {
        assert_eq!(axis_range(0, 1216, 0, 1200), (0, 1216));
        assert_eq!(axis_range(-100, 100, 0, 9), (-100, 100));
        // Degenerate logical ranges fall back.
        assert_eq!(axis_range(0, 0, 0, 1200), (0, 1200));
        assert_eq!(axis_range(500, 10, 0, 1200), (0, 1200));
    }

    #[test]
    fn axis_from_cap_applies_fallback() {
        let c = cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, 0), (0, 3000));
        let a = Axis::from_cap(&c);
        assert_eq!((a.min, a.max), (0, 3000));
        let c = cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, 4095), (0, 3000));
        let a = Axis::from_cap(&c);
        assert_eq!((a.min, a.max), (0, 4095));
    }

    #[test]
    fn decode_unsigned_passes_value_through() {
        let a = Axis::from_cap(&ValueCap {
            bit_size: 12,
            ..cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, 4095), (0, 0))
        });
        assert_eq!(a.decode(0), 0);
        assert_eq!(a.decode(0xFFF), 4095);
        let wide = Axis::from_cap(&ValueCap {
            bit_size: 32,
            ..cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, i32::MAX), (0, 0))
        });
        assert_eq!(wide.decode(u32::MAX), i32::MAX);
    }

    #[test]
    fn decode_sign_extends_negative_logical_ranges() {
        let a = Axis::from_cap(&ValueCap {
            bit_size: 12,
            ..cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (-2048, 2047), (0, 0))
        });
        assert_eq!(a.decode(0xFFF), -1);
        assert_eq!(a.decode(0x800), -2048);
        assert_eq!(a.decode(0x7FF), 2047);
        assert_eq!(a.decode(5), 5);
        let a32 = Axis::from_cap(&ValueCap {
            bit_size: 32,
            ..cap(
                PAGE_GENERIC_DESKTOP,
                USAGE_X,
                1,
                (i32::MIN, i32::MAX),
                (0, 0),
            )
        });
        assert_eq!(a32.decode(u32::MAX), -1);
        // A nonsense bit size never panics.
        let odd = Axis::from_cap(&ValueCap {
            bit_size: 0,
            ..cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (-5, 5), (0, 0))
        });
        assert_eq!(odd.decode(3), 3);
        let odd = Axis::from_cap(&ValueCap {
            bit_size: 200,
            ..cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (-5, 5), (0, 0))
        });
        assert_eq!(odd.decode(u32::MAX), -1);
    }

    #[test]
    fn layout_picks_lowest_nonzero_link_collection_first() {
        let l = layout(&ptp_caps()).expect("touchpad layout");
        let links: Vec<u16> = l.fingers.iter().map(|f| f.link_collection).collect();
        assert_eq!(links, vec![1, 2, 3, 4, 5]);
        let first = l.fingers[0];
        assert_eq!((first.x.min, first.x.max), (0, 1217));
        assert_eq!((first.y.min, first.y.max), (0, 800));
        assert!(l.has_contact_count);
    }

    #[test]
    fn layout_ignores_top_level_collection_and_other_pages() {
        let mut caps = vec![
            // X/Y at link collection 0 must never be a finger.
            cap(PAGE_GENERIC_DESKTOP, USAGE_X, 0, (0, 10), (0, 0)),
            cap(PAGE_GENERIC_DESKTOP, USAGE_Y, 0, (0, 10), (0, 0)),
            // Digitizer usages with X/Y numbers are not X/Y.
            cap(PAGE_DIGITIZER, USAGE_X, 1, (0, 10), (0, 0)),
            cap(PAGE_DIGITIZER, USAGE_Y, 1, (0, 10), (0, 0)),
        ];
        assert_eq!(layout(&caps), None);
        caps.extend(xy(7, 900, 600));
        let l = layout(&caps).expect("layout");
        assert_eq!(l.fingers.len(), 1);
        assert_eq!(l.fingers[0].link_collection, 7);
        assert_eq!(l.fingers[0].x.max, 900);
        assert!(!l.has_contact_count);
    }

    #[test]
    fn layout_skips_collections_missing_an_axis() {
        let caps = vec![
            cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, 10), (0, 0)), // no Y in 1
            cap(PAGE_GENERIC_DESKTOP, USAGE_Y, 2, (0, 10), (0, 0)), // no X in 2
            cap(PAGE_GENERIC_DESKTOP, USAGE_X, 3, (0, 30), (0, 0)),
            cap(PAGE_GENERIC_DESKTOP, USAGE_Y, 3, (0, 20), (0, 0)),
        ];
        let l = layout(&caps).expect("layout");
        assert_eq!(l.fingers.len(), 1);
        assert_eq!(l.fingers[0].link_collection, 3);
        assert_eq!(layout(&caps[..2]), None);
        assert_eq!(layout(&[]), None);
    }

    #[test]
    fn layout_accepts_range_caps_and_dedups() {
        let range = ValueCap {
            usage_min: USAGE_X,
            usage_max: USAGE_Y,
            ..cap(PAGE_GENERIC_DESKTOP, USAGE_X, 2, (0, 0), (0, 5000))
        };
        // Duplicate X cap in the same collection (e.g. a second report ID).
        let dup = cap(PAGE_GENERIC_DESKTOP, USAGE_X, 2, (0, 77), (0, 0));
        let l = layout(&[range, dup]).expect("layout");
        assert_eq!(l.fingers.len(), 1);
        let f = l.fingers[0];
        assert_eq!(f.link_collection, 2);
        // First matching cap wins; the degenerate logical range falls back.
        assert_eq!((f.x.min, f.x.max), (0, 5000));
        assert_eq!((f.y.min, f.y.max), (0, 5000));
    }

    #[test]
    fn layout_caps_finger_count() {
        let caps: Vec<ValueCap> = (1..=40u16).flat_map(|l| xy(l, 100, 100)).collect();
        let l = layout(&caps).expect("layout");
        assert_eq!(l.fingers.len(), MAX_CONTACTS);
        assert_eq!(l.fingers[0].link_collection, 1);
    }

    #[test]
    fn tip_switch_is_usage_0x42() {
        assert!(tip_on(&[USAGE_TIP_SWITCH]));
        assert!(tip_on(&[0x47, USAGE_TIP_SWITCH])); // confidence + tip
        assert!(!tip_on(&[0x47]));
        assert!(!tip_on(&[]));
    }

    #[test]
    fn single_finger_parallel_mode() {
        let mut t = FrameTracker::default();
        let (u, read) = feed(&mut t, Some(1), 5, &[touch(10, 20)]);
        assert_eq!(u, moved(0, 10, 20));
        // Padding collections past the contact count are not read.
        assert_eq!(read, vec![0]);
        let (u, _) = feed(&mut t, Some(1), 5, &[touch(11, 21)]);
        assert_eq!(u, moved(0, 11, 21));
        // Lift report: the contact comes once more with tip switch off.
        let (u, _) = feed(&mut t, Some(1), 5, &[lifted(11, 21)]);
        assert_eq!(u, Update::Lift);
    }

    #[test]
    fn contact_count_zero_means_no_touch() {
        let mut t = FrameTracker::default();
        assert_eq!(feed(&mut t, Some(1), 5, &[touch(1, 1)]).0, moved(0, 1, 1));
        let (u, read) = feed(&mut t, Some(0), 5, &[touch(9, 9)]);
        assert_eq!(u, Update::Lift);
        assert!(read.is_empty());
        // Repeated zeros stay lifted.
        assert_eq!(feed(&mut t, Some(0), 1, &[touch(9, 9)]).0, Update::Lift);
    }

    #[test]
    fn padding_with_stale_tip_is_ignored() {
        let mut t = FrameTracker::default();
        let (u, read) = feed(&mut t, Some(1), 5, &[lifted(1, 1), touch(50, 50)]);
        assert_eq!(u, Update::Lift);
        assert_eq!(read, vec![0]);
    }

    #[test]
    fn first_touching_contact_drives_parallel_mode() {
        let mut t = FrameTracker::default();
        // Two fingers, both down: the first collection wins.
        let (u, _) = feed(&mut t, Some(2), 5, &[touch(1, 2), touch(3, 4)]);
        assert_eq!(u, moved(0, 1, 2));
        // The first finger lifts while the second stays: no blip to Lift.
        let (u, _) = feed(&mut t, Some(2), 5, &[lifted(1, 2), touch(3, 4)]);
        assert_eq!(u, moved(1, 3, 4));
        // Next frame the remaining finger moves into the first collection.
        let (u, _) = feed(&mut t, Some(1), 5, &[touch(5, 6)]);
        assert_eq!(u, moved(0, 5, 6));
    }

    #[test]
    fn hybrid_mode_continuation_reports_keep_touching() {
        // One finger collection per report, two fingers down.
        let mut t = FrameTracker::default();
        let (u, _) = feed(&mut t, Some(2), 1, &[touch(10, 10)]);
        assert_eq!(u, moved(0, 10, 10));
        // Second report of the frame: count 0, other finger. Not a lift, and
        // it must not move the pad to the second finger.
        let (u, read) = feed(&mut t, Some(0), 1, &[touch(90, 90)]);
        assert_eq!(u, Update::Keep);
        assert_eq!(read, vec![0]);
        // Next frame.
        let (u, _) = feed(&mut t, Some(2), 1, &[touch(12, 12)]);
        assert_eq!(u, moved(0, 12, 12));
        assert_eq!(feed(&mut t, Some(0), 1, &[touch(92, 92)]).0, Update::Keep);
        // Frame complete: a further zero is a real "no contacts".
        assert_eq!(feed(&mut t, Some(0), 1, &[touch(92, 92)]).0, Update::Lift);
    }

    #[test]
    fn hybrid_mode_first_contact_lifting_hands_over() {
        let mut t = FrameTracker::default();
        let (u, _) = feed(&mut t, Some(2), 1, &[lifted(10, 10)]);
        assert_eq!(
            u,
            Update::Keep,
            "frame not finished, another contact follows"
        );
        let (u, _) = feed(&mut t, Some(0), 1, &[touch(90, 90)]);
        assert_eq!(u, moved(0, 90, 90));
    }

    #[test]
    fn hybrid_mode_all_contacts_lifting_ends_in_lift() {
        let mut t = FrameTracker::default();
        assert_eq!(
            feed(&mut t, Some(3), 2, &[lifted(1, 1), lifted(2, 2)]).0,
            Update::Keep
        );
        assert_eq!(
            feed(&mut t, Some(0), 2, &[lifted(3, 3), touch(4, 4)]).0,
            Update::Lift
        );
    }

    #[test]
    fn without_contact_count_each_report_is_a_frame() {
        let mut t = FrameTracker::default();
        let (u, read) = feed(&mut t, None, 3, &[lifted(0, 0), touch(7, 8)]);
        assert_eq!(u, moved(1, 7, 8));
        assert_eq!(read, vec![0, 1, 2]);
        assert_eq!(feed(&mut t, None, 3, &[lifted(0, 0)]).0, Update::Lift);
        assert_eq!(feed(&mut t, None, 1, &[touch(1, 1)]).0, moved(0, 1, 1));
    }

    #[test]
    fn unreadable_contact_counts_as_lifted() {
        let mut t = FrameTracker::default();
        assert_eq!(feed(&mut t, Some(1), 1, &[None]).0, Update::Lift);
        assert_eq!(
            feed(&mut t, Some(2), 2, &[None, touch(4, 5)]).0,
            moved(1, 4, 5)
        );
    }

    #[test]
    fn garbage_contact_count_cannot_stall_lift() {
        let mut t = FrameTracker::default();
        assert_eq!(
            feed(&mut t, Some(u32::MAX), 1, &[touch(1, 1)]).0,
            moved(0, 1, 1)
        );
        // Zeros are continuations until the clamped frame is used up, then lift.
        let mut lifted_after = None;
        for n in 1..=MAX_CONTACTS + 1 {
            if feed(&mut t, Some(0), 1, &[lifted(0, 0)]).0 == Update::Lift {
                lifted_after = Some(n);
                break;
            }
        }
        assert!(lifted_after.is_some_and(|n| n <= MAX_CONTACTS));
    }

    #[test]
    fn no_finger_collections_is_inert() {
        let mut t = FrameTracker::default();
        let (u, read) = feed(&mut t, Some(1), 0, &[touch(1, 1)]);
        assert_eq!(u, Update::Keep);
        assert!(read.is_empty());
        assert_eq!(t, FrameTracker::default());
        // More finger collections than the cap are never read.
        let (_, read) = feed(&mut t, None, 50, &[]);
        assert_eq!(read.len(), MAX_CONTACTS);
    }

    #[test]
    fn hybrid_two_collections_three_contacts() {
        let mut t = FrameTracker::default();
        let (u, read) = feed(&mut t, Some(3), 2, &[lifted(1, 1), lifted(2, 2)]);
        assert_eq!(u, Update::Keep);
        assert_eq!(read, vec![0, 1]);
        // Last report of the frame: one real contact, then padding.
        let (u, read) = feed(&mut t, Some(0), 2, &[touch(3, 3), touch(99, 99)]);
        assert_eq!(u, moved(0, 3, 3));
        assert_eq!(read, vec![0]);
    }

    /// Two fingers with different ranges: finger 1 has X 0..1000, Y 0..500.
    fn two_finger_layout() -> Layout {
        let caps: Vec<ValueCap> = [xy(1, 2000, 1000), xy(2, 1000, 500)].concat();
        layout(&caps).expect("layout")
    }

    fn assert_touch(change: PadChange, x: f32, y: f32) {
        match change {
            PadChange::Touch { x: cx, y: cy } => {
                assert!((cx - x).abs() < 1e-6, "x {cx} != {x}");
                assert!((cy - y).abs() < 1e-6, "y {cy} != {y}");
            }
            other => panic!("expected Touch({x}, {y}), got {other:?}"),
        }
    }

    #[test]
    fn pad_change_normalises_and_flips_y() {
        let l = two_finger_layout();
        // HID y = min is the top edge of the pad: our y = 1.
        assert_touch(pad_change(&l, moved(0, 500, 0)), 0.25, 1.0);
        assert_touch(pad_change(&l, moved(0, 2000, 1000)), 1.0, 0.0);
        assert_touch(pad_change(&l, moved(0, 0, 250)), 0.0, 0.75);
        // Each finger collection uses its own range.
        assert_touch(pad_change(&l, moved(1, 500, 125)), 0.5, 0.75);
        // Out-of-range values clamp.
        assert_touch(pad_change(&l, moved(1, -40, 9000)), 0.0, 0.0);
        assert_touch(pad_change(&l, moved(1, i32::MAX, i32::MIN)), 1.0, 1.0);
    }

    #[test]
    fn unit_used_by_pad_change_clamps_and_guards_nan() {
        assert_eq!(crate::unit(0.25), 0.25);
        assert_eq!(crate::unit(-1.0), 0.0);
        assert_eq!(crate::unit(7.0), 1.0);
        assert_eq!(crate::unit(f32::INFINITY), 1.0);
        assert_eq!(crate::unit(f32::NEG_INFINITY), 0.0);
        assert_eq!(crate::unit(f32::NAN), 0.5);
    }

    #[test]
    fn pad_change_degenerate_range_is_centre() {
        let caps = [
            cap(PAGE_GENERIC_DESKTOP, USAGE_X, 1, (0, 0), (0, 0)),
            cap(PAGE_GENERIC_DESKTOP, USAGE_Y, 1, (9, 9), (5, 5)),
        ];
        let l = layout(&caps).expect("layout");
        assert_touch(pad_change(&l, moved(0, 123, -7)), 0.5, 0.5);
    }

    #[test]
    fn pad_change_passes_keep_and_lift_and_ignores_bad_finger() {
        let l = two_finger_layout();
        assert_eq!(pad_change(&l, Update::Keep), PadChange::Keep);
        assert_eq!(pad_change(&l, Update::Lift), PadChange::Lift);
        assert_eq!(pad_change(&l, moved(2, 1, 1)), PadChange::Keep);
        assert_eq!(pad_change(&l, moved(usize::MAX, 1, 1)), PadChange::Keep);
    }

    #[test]
    fn stale_touch_lifts_only_a_touching_pad_after_150_ms() {
        let ms = Duration::from_millis;
        assert_eq!(STALE_TOUCH, ms(150));
        assert!(!touch_is_stale(true, Some(ms(0))));
        assert!(!touch_is_stale(true, Some(ms(149))));
        assert!(touch_is_stale(true, Some(ms(150))));
        assert!(touch_is_stale(true, Some(Duration::MAX)));
        assert!(touch_is_stale(true, None));
        // Already lifted: nothing to do.
        assert!(!touch_is_stale(false, Some(ms(10_000))));
        assert!(!touch_is_stale(false, None));
    }

    #[test]
    fn lost_continuation_is_cleared_by_the_watchdog() {
        // Hybrid frame announces 2 contacts; the second report is lost and
        // the pad stays touching (the tracker cannot tell a lost report
        // from a slow one). The watchdog is what lifts it.
        let mut t = FrameTracker::default();
        assert_eq!(feed(&mut t, Some(2), 1, &[touch(1, 1)]).0, moved(0, 1, 1));
        assert_eq!(feed(&mut t, Some(0), 1, &[lifted(0, 0)]).0, Update::Keep);
        assert!(touch_is_stale(true, Some(STALE_TOUCH)));
        // After the watchdog resets the tracker, a zero is a real lift.
        t = FrameTracker::default();
        assert_eq!(feed(&mut t, Some(0), 1, &[touch(9, 9)]).0, Update::Lift);
    }
}
