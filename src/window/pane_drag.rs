//! App-owned pane grips and drag gestures, independent of header mods.
use super::*;
use crate::layout::Edge;

pub(super) struct PaneDrag {
    pane: String,
    tab: String,
    down: Point<Pixels>,
    moving: bool,
    target: Option<(String, Option<Edge>)>,
}

/// The outer quarter of a pane chooses its nearest edge; the middle swaps.
fn drop_zone(bounds: Bounds<Pixels>, at: Point<Pixels>) -> Option<Option<Edge>> {
    if !bounds.contains(&at) || bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
        return None;
    }
    let x = f32::from(at.x - bounds.origin.x) / f32::from(bounds.size.width);
    let y = f32::from(at.y - bounds.origin.y) / f32::from(bounds.size.height);
    let (distance, edge) = [(x, Edge::Left), (1. - x, Edge::Right), (y, Edge::Top), (1. - y, Edge::Bottom)]
        .into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
    Some((distance < 0.25).then_some(edge))
}

/// The top row of a pane without a top header.
const STRIP: f32 = 18.;

fn inherited_movable(root: &Node, id: &str) -> Option<bool> {
    root.path_to(id)?.iter().rev().find_map(|id| root.find(id)?.movable)
}

impl AppWindow {
    pub(super) fn pane_movable(&self, pane: &str) -> bool {
        let Some(tab) = self.model.active_tab() else { return false };
        if tab.zoomed.is_some() || tab.root.panes().len() < 2 || !tab.root.find(pane).is_some_and(Node::is_pane) {
            return false;
        }
        let owner = self.chrome.visible_blocks().into_iter().find(|n| matches!(n.content(), Some(Content::Context { .. })));
        inherited_movable(&tab.root, pane)
            .or_else(|| owner.and_then(|n| inherited_movable(&self.chrome, &n.id)))
            .unwrap_or(true)
    }

    /// Where the pane's header ends, for a Cmd-drag on it. No grip is drawn any more.
    pub(super) fn pane_grip(&self, pane: &str, header: Option<f32>, _cx: &mut Context<Self>) -> Option<AnyElement> {
        let row = header.unwrap_or(STRIP).max(STRIP);
        self.pane_grip_rows.borrow_mut().insert(pane.to_owned(), row);
        None
    }

    /// A pane whose header holds this point: Cmd-dragging there moves the pane.
    fn pane_header_at(&self, at: Point<Pixels>) -> Option<String> {
        let tab = self.model.active_tab()?;
        if tab.zoomed.is_some() { return None; }
        let bounds = self.bounds.borrow();
        let rows = self.pane_grip_rows.borrow();
        tab.shown().visible_blocks().into_iter().find_map(|n| {
            let b = *bounds.get(&n.id)?;
            let row = rows.get(&n.id).copied().unwrap_or(STRIP);
            let inside = at.x >= b.origin.x && at.x <= b.origin.x + b.size.width
                && at.y >= b.origin.y && at.y <= b.origin.y + settings::z(row);
            (inside && self.pane_movable(&n.id)).then(|| n.id.clone())
        })
    }

    pub(super) fn pane_press(&mut self, pane: &str, at: Point<Pixels>, cx: &mut Context<Self>) {
        if self.pane_movable(pane) {
            self.pane_drag = Some(PaneDrag {
                pane: pane.to_owned(), tab: self.model.active_tab().unwrap().id.clone(),
                down: at, moving: false, target: None,
            });
            cx.notify();
        }
    }

    fn pane_target(&self, at: Point<Pixels>) -> Option<(String, Option<Edge>)> {
        let d = self.pane_drag.as_ref()?;
        let tab = self.model.active_tab().filter(|t| t.id == d.tab && t.zoomed.is_none())?;
        let bounds = self.bounds.borrow();
        tab.shown().visible_blocks().into_iter().filter(|n| n.is_pane() && n.id != d.pane)
            .find_map(|n| Some((n.id.clone(), drop_zone(*bounds.get(&n.id)?, at)?)))
    }

    fn pane_move(&mut self, at: Point<Pixels>, pressed: bool, cx: &mut Context<Self>) {
        let Some(d) = self.pane_drag.as_mut() else { return };
        if !pressed {
            self.cancel_pane_drag(cx);
            return;
        }
        if !d.moving && (at.x - d.down.x).abs().max((at.y - d.down.y).abs()) < settings::z(4.) {
            return;
        }
        d.moving = true;
        let target = self.pane_target(at);
        self.pane_drag.as_mut().unwrap().target = target;
        cx.notify();
    }

    pub(super) fn cancel_pane_drag(&mut self, cx: &mut Context<Self>) {
        if self.pane_drag.take().is_some() {
            cx.notify();
        }
    }

    fn pane_release(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let target = self.pane_target(at);
        let Some(d) = self.pane_drag.take() else { return false };
        if !d.moving {
            cx.notify();
            return true;
        }
        if self.pane_movable(&d.pane)
            && let Some((target, edge)) = target
            && self.model.move_pane(&d.pane, &target, edge)
        {
            self.sync(window, cx);
        }
        cx.defer_in(window, |this, window, cx| this.restore_focus(window, cx));
        cx.notify();
        true
    }

    pub(super) fn pane_drag_layer(&self, cx: &mut Context<Self>) -> AnyElement {
        let preview = self.pane_drag.as_ref().and_then(|d| d.target.as_ref()).and_then(|(id, edge)| {
            let mut b = *self.bounds.borrow().get(id)?;
            match edge {
                Some(Edge::Left) => b.size.width /= 2.,
                Some(Edge::Right) => { b.size.width /= 2.; b.origin.x += b.size.width; }
                Some(Edge::Top) => b.size.height /= 2.,
                Some(Edge::Bottom) => { b.size.height /= 2.; b.origin.y += b.size.height; }
                None => {}
            }
            Some(div().absolute().left(b.origin.x).top(b.origin.y).w(b.size.width).h(b.size.height)
                .bg(c("accent").opacity(0.18)).border_2().border_color(c("accent")).into_any_element())
        });
        let this = cx.entity().downgrade();
        div().id("pane-drag-layer").absolute().inset_0()
            .when(self.pane_drag.is_some(), |d| d.occlude().cursor(CursorStyle::ClosedHand))
            .children(preview).child(canvas(
            |_, _, _| {},
            move |_, _, window, _| {
                let t = this.clone();
                window.on_mouse_event(move |ev: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture {
                        let _ = t.update(cx, |this, cx| {
                            this.pane_move(ev.position, ev.pressed_button == Some(MouseButton::Left), cx);
                        });
                    }
                });
                // Cmd over a pane's header starts the same drag the grip used to.
                let t = this.clone();
                window.on_mouse_event(move |ev: &MouseDownEvent, phase, _, cx| {
                    if phase != DispatchPhase::Capture || ev.button != MouseButton::Left || !ev.modifiers.platform {
                        return;
                    }
                    let _ = t.update(cx, |this, cx| {
                        // Under a popup or a floating mod its own press wins.
                        if !this.overlays.is_empty() || !this.float_hover.is_empty() { return; }
                        if let Some(pane) = this.pane_header_at(ev.position) {
                            this.pane_press(&pane, ev.position, cx);
                            cx.stop_propagation();
                        }
                    });
                });
                let t = this.clone();
                window.on_mouse_event(move |ev: &MouseUpEvent, phase, window, cx| {
                    if phase == DispatchPhase::Capture && ev.button == MouseButton::Left
                        && t.update(cx, |this, cx| this.pane_release(ev.position, window, cx)).unwrap_or(false)
                    { cx.stop_propagation(); }
                });
            },
        ).absolute().size_full()).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::{drop_zone, inherited_movable};
    use crate::layout::{Axis, Content, Edge, Node};
    use gpui::{Bounds, point, px, size};
    #[test]
    fn zones_are_relative_to_the_pane_and_outside_cancels() {
        let b = Bounds::new(point(px(100.), px(200.)), size(px(800.), px(400.)));
        for (x, y, expected) in [(500., 400., Some(None)), (110., 400., Some(Some(Edge::Left))),
            (890., 400., Some(Some(Edge::Right))), (500., 210., Some(Some(Edge::Top))),
            (500., 590., Some(Some(Edge::Bottom))), (99., 400., None)] {
            assert_eq!(drop_zone(b, point(px(x), px(y))), expected);
        }
    }
    #[test]
    fn movable_uses_the_nearest_explicit_ancestor() {
        let mut root = Node::container("root", Axis::Row, vec![Node::container("inner", Axis::Column,
            vec![Node::block("pane", Content::Terminal { terminal: "t1".into() })])]);
        assert_eq!(inherited_movable(&root, "pane"), None);
        root.movable = Some(false);
        assert_eq!(inherited_movable(&root, "pane"), Some(false));
        root.find_mut("inner").unwrap().movable = Some(true);
        assert_eq!(inherited_movable(&root, "pane"), Some(true));
    }
}
