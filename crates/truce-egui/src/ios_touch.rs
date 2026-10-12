//! UI-thread touch translation, independent of UIKit for regression testing.
//! UIKit positions are already local logical points (safe-area offset and Retina
//! scale must not be applied again). Only egui's zoom changes their coordinate space.
#![cfg_attr(not(target_os = "ios"), allow(dead_code))]
use egui::{Event, Modifiers, PointerButton, Pos2, TouchDeviceId, TouchId, TouchPhase};

#[derive(Default)]
pub(crate) struct TouchTranslator {
    contacts: Vec<Contact>,
    next_id: u64,
    primary: Option<u64>,
}
struct Contact {
    native: usize,
    id: u64,
    pos: Pos2,
}

impl TouchTranslator {
    /// Sort each UIKit changed-touch set by its native identity before assigning
    /// monotonically increasing IDs. The primary stays fixed for the entire chord:
    /// lifting it releases the pointer; secondary fingers never become extra clicks.
    pub(crate) fn translate(
        &mut self,
        phase: TouchPhase,
        changed: &mut [(usize, Pos2)],
        out: &mut Vec<Event>,
    ) {
        changed.sort_unstable_by_key(|(native, _)| *native);
        for &(native, pos) in changed.iter() {
            if phase == TouchPhase::Start {
                if self.contacts.iter().any(|c| c.native == native) {
                    continue;
                }
                self.next_id += 1;
                if self.contacts.is_empty() {
                    self.primary = Some(self.next_id);
                }
                self.contacts.push(Contact {
                    native,
                    id: self.next_id,
                    pos,
                });
            }
            let Some(index) = self.contacts.iter().position(|c| c.native == native) else {
                continue;
            };
            let contact = &mut self.contacts[index];
            contact.pos = pos;
            out.push(Event::Touch {
                device_id: TouchDeviceId(0),
                id: TouchId(contact.id),
                phase,
                pos,
                force: None,
            });
            if self.primary == Some(contact.id) {
                out.push(Event::PointerMoved(pos));
                if phase != TouchPhase::Move {
                    out.push(Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed: phase == TouchPhase::Start,
                        modifiers: Modifiers::default(),
                    });
                }
                if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                    out.push(Event::PointerGone);
                    self.primary = None;
                }
            }
            if matches!(phase, TouchPhase::End | TouchPhase::Cancel) {
                self.contacts.remove(index);
            }
        }
    }

    pub(crate) fn cancel_all(&mut self, out: &mut Vec<Event>) {
        let mut changed: Vec<_> = self.contacts.iter().map(|c| (c.native, c.pos)).collect();
        self.translate(TouchPhase::Cancel, &mut changed, out);
    }
}

pub(crate) fn zoom_position(pos: Pos2, zoom: f32) -> Pos2 {
    pos / zoom.max(0.01)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn apply(t: &mut TouchTranslator, phase: TouchPhase, ids: &[usize]) -> Vec<Event> {
        let mut events = Vec::new();
        let mut changed: Vec<_> = ids
            .iter()
            .map(|&id| (id, egui::pos2(id as f32, 40.0)))
            .collect();
        t.translate(phase, &mut changed, &mut events);
        events
    }
    fn touches(events: &[Event]) -> Vec<(u64, TouchPhase)> {
        events
            .iter()
            .filter_map(|e| {
                if let Event::Touch { id, phase, .. } = e {
                    Some((id.0, *phase))
                } else {
                    None
                }
            })
            .collect()
    }
    fn buttons(events: &[Event]) -> Vec<bool> {
        events
            .iter()
            .filter_map(|e| {
                if let Event::PointerButton { pressed, .. } = e {
                    Some(*pressed)
                } else {
                    None
                }
            })
            .collect()
    }
    #[test]
    fn unordered_three_fingers_keep_ids_and_primary_until_lift() {
        let mut t = TouchTranslator::default();
        let starts = apply(&mut t, TouchPhase::Start, &[30, 10, 20]);
        assert_eq!(
            touches(&starts),
            vec![
                (1, TouchPhase::Start),
                (2, TouchPhase::Start),
                (3, TouchPhase::Start)
            ]
        );
        assert_eq!(buttons(&starts), vec![true]);
        assert_eq!(
            touches(&apply(&mut t, TouchPhase::Move, &[30, 10])),
            vec![(1, TouchPhase::Move), (3, TouchPhase::Move)]
        );
        assert!(buttons(&apply(&mut t, TouchPhase::End, &[20])).is_empty());
        assert_eq!(buttons(&apply(&mut t, TouchPhase::End, &[10])), vec![false]);
        assert!(
            apply(&mut t, TouchPhase::Move, &[30])
                .iter()
                .all(|e| matches!(e, Event::Touch { .. }))
        );
        assert!(buttons(&apply(&mut t, TouchPhase::Start, &[40])).is_empty());
        apply(&mut t, TouchPhase::End, &[40, 30]);
        assert_eq!(
            buttons(&apply(&mut t, TouchPhase::Start, &[50])),
            vec![true]
        );
    }
    #[test]
    fn cancellation_releases_once_and_reopen_starts_clean() {
        let mut t = TouchTranslator::default();
        apply(&mut t, TouchPhase::Start, &[2, 1]);
        let mut out = Vec::new();
        t.cancel_all(&mut out);
        assert_eq!(
            touches(&out),
            vec![(1, TouchPhase::Cancel), (2, TouchPhase::Cancel)]
        );
        assert_eq!(buttons(&out), vec![false]);
        assert!(apply(&mut t, TouchPhase::End, &[1, 2]).is_empty());
        assert_eq!(
            touches(&apply(&mut t, TouchPhase::Start, &[1])),
            vec![(3, TouchPhase::Start)]
        );
    }
    #[test]
    fn unordered_batches_and_native_pointer_reuse_preserve_lifecycles() {
        let mut a = TouchTranslator::default();
        let mut b = TouchTranslator::default();
        assert_eq!(
            touches(&apply(&mut a, TouchPhase::Start, &[9, 3])),
            touches(&apply(&mut b, TouchPhase::Start, &[3, 9]))
        );
        let moved = apply(&mut a, TouchPhase::Move, &[9, 3]);
        assert!(
            matches!(moved[0], Event::Touch { id: TouchId(1), pos, .. } if pos == egui::pos2(3.0, 40.0))
        );
        assert!(matches!(moved[1], Event::PointerMoved(pos) if pos == egui::pos2(3.0, 40.0)));
        assert_eq!(
            touches(&apply(&mut a, TouchPhase::Cancel, &[9])),
            vec![(2, TouchPhase::Cancel)]
        );
        assert_eq!(
            touches(&apply(&mut a, TouchPhase::Start, &[9])),
            vec![(3, TouchPhase::Start)]
        );
        assert!(buttons(&apply(&mut a, TouchPhase::Start, &[9])).is_empty());
        let mut reopened = TouchTranslator::default();
        assert_eq!(
            buttons(&apply(&mut reopened, TouchPhase::Start, &[9])),
            vec![true]
        );
    }

    #[test]
    fn local_safe_area_points_use_inverse_zoom_without_retina_rescaling() {
        assert_eq!(
            zoom_position(egui::pos2(30.0, 60.0), 0.5),
            egui::pos2(60.0, 120.0)
        );
        assert_eq!(
            zoom_position(egui::pos2(30.0, 60.0), 1.0),
            egui::pos2(30.0, 60.0)
        );
    }
}
