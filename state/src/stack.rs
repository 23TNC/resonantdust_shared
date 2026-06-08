//! Stack-movement model — the sans-IO core behind `place_card`.
//!
//! "Put this card at this position" — stacked onto a parent (in a branch
//! direction) or loose at an explicit address — validated and resolved over an
//! abstract [`StackStore`], returning a [`Plan`] of row writes the caller
//! applies. No IO: the server impls [`StackStore`] over its db and applies the
//! plan via `update_with_at`; the client impls it over its world model and
//! applies the plan optimistically (predicting the move) while the gate's
//! authoritative `place_card` round-trips. One implementation, no drift between
//! the shard reducer and the client's mirror.
//!
//! Flat-chain model (mirror of `Micro` in `resonantdust_codec::card_model`): a
//! chain is one loose/snapped **root** plus N **members** whose `micro_location`
//! points at the root, each carrying a `branch` + `index`. Stacking a card makes
//! it (and its own members) members of the target's root.

use resonantdust_codec::card_model::{
    drop_hold_count, hold_count, is_dead, stack_branch, stack_index, HoldField, Micro,
};
use resonantdust_codec::packed::{
    owner_of, surface_of, with_surface, STACK_DIR_DOWN, STACK_DIR_HEX, STACK_DIR_UP,
};

use crate::recipe_state::{owning_player, CardStore, CardView, WORLD_PLAYER_ID};

/// The reads stack movement needs beyond [`CardStore`]: the members of a chain
/// root (cards whose `micro_location == root` and that are stack members),
/// current at `now_ms`.
pub trait StackStore: CardStore {
    fn members_of(&self, root_id: u32, now_ms: u64) -> Vec<CardView>;
}

/// Where a placement puts the source card. The Rust-enum counterpart of the
/// shard's flat wire `Placement` — the client/gate map to/from the wire form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Stack the source as a member of `parent_id`'s chain in `direction`
    /// (`STACK_DIR_UP`/`DOWN`/`HEX`).
    Stack { parent_id: u32, direction: u8 },
    /// Place the source loose at an explicit address. `q`/`r` are the world cell
    /// (0 for container surfaces); `x`/`y` the within-cell offset.
    Loose {
        surface: u8,
        macro_zone: u64,
        q: u8,
        r: u8,
        x: i16,
        y: i16,
    },
}

/// One row the plan writes: the card's new `(macro_zone, micro)`. `owner_id` is
/// never part of a placement — ownership is independent of position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Write {
    pub card_id: u32,
    pub macro_zone: u64,
    pub micro: Micro,
}

/// The full set of writes a placement produces: the source, then its members
/// (re-rooted on a stack move, or just travelling on a loose move).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub writes: Vec<Write>,
}

/// Validate + resolve a placement of `card_id` by `caller_player_id`, returning
/// the [`Plan`] to apply, or an `Err` describing why it's rejected. Pure over
/// `store`; the caller does the actual row writes.
pub fn plan_place<S: StackStore>(
    store: &S,
    card_id: u32,
    placement: Placement,
    caller_player_id: u32,
    now_ms: u64,
) -> Result<Plan, String> {
    // ── source eligibility ──────────────────────────────────────────────
    let source = store
        .card_at(card_id, now_ms)
        .ok_or_else(|| format!("place: card {card_id} not found"))?;
    if is_dead(source.flags) {
        return Err(format!("place: card {card_id} is dead"));
    }
    let source_owner = owning_player(store, card_id, now_ms).unwrap_or(WORLD_PLAYER_ID);
    if source_owner != caller_player_id {
        return Err(format!(
            "place: card {card_id} is owned by player {source_owner} (not {caller_player_id})"
        ));
    }
    held_check(&source, "card")?;

    // Members travel with the source; validate the same way up front.
    let members = store.members_of(card_id, now_ms);
    for m in &members {
        held_check(m, "descendant")?;
    }

    // ── resolve the destination ─────────────────────────────────────────
    let (surface, macro_zone, micro) = match placement {
        Placement::Stack { parent_id, direction } => {
            resolve_stack(store, card_id, parent_id, direction, caller_player_id, now_ms)?
        }
        Placement::Loose { surface, macro_zone, q, r, x, y } => {
            resolve_loose(store, caller_player_id, surface, macro_zone, q, r, x, y, now_ms)?
        }
    };
    let full_macro = with_surface(macro_zone, surface);

    // ── build the write plan ────────────────────────────────────────────
    let mut writes = Vec::with_capacity(1 + members.len());
    writes.push(Write { card_id, macro_zone: full_macro, micro });
    for m in &members {
        // Stack move: members re-root onto the source's new root (keeping their
        // own branch/index). Loose move: source stays root, members just travel.
        let m_micro = Micro::of(m.micro_location, m.flags);
        let new_micro = match micro {
            Micro::Stacked { root: new_root, .. } => match m_micro {
                Micro::Stacked { branch, index, .. } => Micro::Stacked { root: new_root, branch, index },
                loose => loose,
            },
            Micro::Loose { .. } => m_micro,
        };
        writes.push(Write { card_id: m.card_id, macro_zone: full_macro, micro: new_micro });
    }
    Ok(Plan { writes })
}

/// Reject if `view` is held by an in-flight action (claim / borrow / position).
fn held_check(view: &CardView, what: &str) -> Result<(), String> {
    if hold_count(view.flags, HoldField::SlotClaim) > 0 {
        return Err(format!("place: {what} {} is exclusively held by an in-flight action", view.card_id));
    }
    if hold_count(view.flags, HoldField::SlotBorrow) > 0 {
        return Err(format!("place: {what} {} is borrow-held by an in-flight action", view.card_id));
    }
    if hold_count(view.flags, HoldField::PositionHold) > 0 {
        return Err(format!("place: {what} {} is position-held by an in-flight action", view.card_id));
    }
    Ok(())
}

/// The chain root of `view`: `micro_location` if it's a stack member, else itself.
pub fn chain_root_of(view: &CardView) -> u32 {
    let micro = Micro::of(view.micro_location, view.flags);
    match micro {
        Micro::Stacked { root, .. } => root,
        Micro::Loose { .. } => view.card_id,
    }
}

/// The **destroy** side of [`plan_place`]'s member re-root: compute the writes to
/// SPLICE the members of each destroyed stack root. Destroying a root would
/// orphan its members (their `micro_location` still points at it); for each
/// destroyed card that is a root with live members, promote one member to a LOOSE
/// root at the destroyed cell and re-root the rest onto it (keeping branch/index).
/// Successor preference: up-branch first (the card on the root), then hex, then
/// down; lowest index within. Same [`Write`] shape as `plan_place`, applied by the
/// caller @completion. A destroyed card that is itself a member (the flat
/// root-not-parent model gives it no members) yields nothing. Pure over `store` —
/// the gate runs it on its gathered snapshot, a client could on its world.
pub fn plan_splice<S: StackStore>(store: &S, destroyed: &[u32], now_ms: u64) -> Vec<Write> {
    use std::collections::BTreeSet;
    let gone: BTreeSet<u32> = destroyed.iter().copied().collect();
    let rank = |b: u8| -> u8 {
        if b == STACK_DIR_UP {
            0
        } else if b == STACK_DIR_HEX {
            1
        } else {
            2
        }
    };
    let mut writes = Vec::new();
    for &root_id in destroyed {
        let Some(root) = store.card_at(root_id, now_ms) else {
            continue;
        };
        let mut members: Vec<CardView> = store
            .members_of(root_id, now_ms)
            .into_iter()
            .filter(|m| !gone.contains(&m.card_id) && !is_dead(m.flags))
            .collect();
        if members.is_empty() {
            continue;
        }
        members.sort_by_key(|m| (rank(stack_branch(m.flags)), stack_index(m.flags), m.card_id));
        // Promote the successor to a LOOSE root at the destroyed root's cell.
        let new_root = members[0].card_id;
        writes.push(Write {
            card_id: new_root,
            macro_zone: root.macro_zone,
            micro: Micro::of(root.micro_location, root.flags),
        });
        // Re-root the rest onto the new root, keeping their branch/index.
        for m in &members[1..] {
            writes.push(Write {
                card_id: m.card_id,
                macro_zone: root.macro_zone,
                micro: Micro::Stacked {
                    root: new_root,
                    branch: stack_branch(m.flags),
                    index: stack_index(m.flags),
                },
            });
        }
    }
    writes
}

/// Next free `index` in `(root, direction)` at `macro_zone` — `max occupied + 1`
/// (saturating at 15), or 0 when the branch is empty. Gap-tolerant.
fn next_branch_index<S: StackStore>(
    store: &S,
    root: u32,
    macro_zone: u64,
    direction: u8,
    now_ms: u64,
) -> u8 {
    let mut max: Option<u8> = None;
    for m in store.members_of(root, now_ms) {
        if m.macro_zone != macro_zone {
            continue;
        }
        if stack_branch(m.flags) == direction {
            let idx = stack_index(m.flags);
            max = Some(max.map_or(idx, |cur| cur.max(idx)));
        }
    }
    match max {
        Some(m) => ((m as u16 + 1).min(15)) as u8,
        None => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_stack<S: StackStore>(
    store: &S,
    source_id: u32,
    parent_id: u32,
    direction: u8,
    caller_player_id: u32,
    now_ms: u64,
) -> Result<(u8, u64, Micro), String> {
    if parent_id == 0 {
        return Err("place: Stack placement with parent_id == 0".to_string());
    }
    if parent_id == source_id {
        return Err(format!("place: card {source_id} can't stack onto itself"));
    }
    if !matches!(direction, STACK_DIR_UP | STACK_DIR_DOWN | STACK_DIR_HEX) {
        return Err(format!("place: invalid direction {direction} (expected UP=1, DOWN=2, HEX=0)"));
    }
    let parent = store
        .card_at(parent_id, now_ms)
        .ok_or_else(|| format!("place: parent card {parent_id} not found"))?;
    if is_dead(parent.flags) {
        return Err(format!("place: parent card {parent_id} is dead"));
    }
    if drop_hold_count(parent.flags) > 0 {
        return Err(format!("place: parent card {parent_id} blocks stacking (drop_hold_count > 0)"));
    }
    let parent_root = chain_root_of(&parent);
    if parent_root == source_id {
        return Err(format!(
            "place: stacking would form a cycle (source {source_id} is the root of parent {parent_id}'s chain)"
        ));
    }
    let chain_player = owning_player(store, parent_root, now_ms).unwrap_or(WORLD_PLAYER_ID);
    if chain_player != WORLD_PLAYER_ID && chain_player != caller_player_id {
        return Err(format!(
            "place: parent {parent_id}'s chain is owned by player {chain_player} (not {caller_player_id})"
        ));
    }
    let index = next_branch_index(store, parent_root, parent.macro_zone, direction, now_ms);
    Ok((
        surface_of(parent.macro_zone),
        parent.macro_zone,
        Micro::Stacked { root: parent_root, branch: direction, index },
    ))
}

#[allow(clippy::too_many_arguments)]
fn resolve_loose<S: StackStore>(
    store: &S,
    caller_player_id: u32,
    surface: u8,
    macro_zone: u64,
    q: u8,
    r: u8,
    x: i16,
    y: i16,
    now_ms: u64,
) -> Result<(u8, u64, Micro), String> {
    // Uniform hex placement on every surface — no per-surface geometry. A
    // container target (a `macro_zone` whose owner band names a card) must be
    // owned by the caller; the world (owner `0`) is open. Snapped-vs-free is a
    // render concern, so the `(x, y)` offset just rides through (the headless
    // client passes `0`, i.e. centred).
    if q >= 8 || r >= 8 {
        return Err(format!("place: cell ({q}, {r}) out of range (0..=7 each)"));
    }
    let owner = owner_of(macro_zone);
    if owner != 0
        && owning_player(store, owner, now_ms).unwrap_or(WORLD_PLAYER_ID) != caller_player_id
    {
        return Err(format!(
            "place: container {macro_zone} (owner {owner}) not owned by caller {caller_player_id}"
        ));
    }
    Ok((surface, macro_zone, Micro::Loose { local_q: q, local_r: r, x, y }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use resonantdust_codec::packed::{with_owner, INVENTORY_LAYER, PLAYER_SOUL_PACKED, WORLD_LAYER};
    use std::collections::HashMap;

    // Build a CardView via the codec so flags are laid out exactly as on the wire.
    fn loose(card_id: u32, owner_id: u32, surface: u8, q: u8, r: u8) -> CardView {
        let (micro_location, flags) = Micro::snap(q, r).apply(0);
        let macro_zone = with_surface(0, surface);
        CardView { card_id, owner_id, micro_location, macro_zone, packed_definition: 0, flags, stock: 0 }
    }
    fn stacked(card_id: u32, owner_id: u32, macro_zone: u64, root: u32, branch: u8, index: u8) -> CardView {
        let (micro_location, flags) = Micro::Stacked { root, branch, index }.apply(0);
        CardView { card_id, owner_id, micro_location, macro_zone, packed_definition: 0, flags, stock: 0 }
    }
    /// A player_soul (identified by definition 0xFFFF): owner_id is the player_id,
    /// terminates the owner walk.
    fn soul(card_id: u32, player_id: u32) -> CardView {
        let (micro_location, flags) = Micro::snap(0, 0).apply(0);
        CardView {
            card_id,
            owner_id: player_id,
            micro_location,
            macro_zone: 0,
            packed_definition: PLAYER_SOUL_PACKED,
            flags,
            stock: 0,
        }
    }

    #[derive(Default)]
    struct Mock(HashMap<u32, CardView>);
    impl Mock {
        fn with(mut self, v: CardView) -> Self {
            self.0.insert(v.card_id, v);
            self
        }
    }
    impl CardStore for Mock {
        fn card_at(&self, id: u32, _t: u64) -> Option<CardView> {
            self.0.get(&id).cloned()
        }
    }
    impl StackStore for Mock {
        fn members_of(&self, root_id: u32, _now: u64) -> Vec<CardView> {
            self.0
                .values()
                .filter(|v| matches!(Micro::of(v.micro_location, v.flags), Micro::Stacked { root, .. } if root == root_id))
                .cloned()
                .collect()
        }
    }

    const P: u32 = 7; // caller player_id

    #[test]
    fn stack_onto_loose_root_picks_first_index() {
        // soul(1024) owns parent(1025, loose on world) and source(1026, loose).
        let store = Mock::default()
            .with(soul(1024, P))
            .with({ let mut c = loose(1025, 1024, WORLD_LAYER, 1, 1); c.macro_zone = with_surface(0, WORLD_LAYER); c })
            .with({ let mut c = loose(1026, 1024, WORLD_LAYER, 2, 2); c.macro_zone = with_surface(0, WORLD_LAYER); c });
        let plan = plan_place(&store, 1026, Placement::Stack { parent_id: 1025, direction: STACK_DIR_UP }, P, 0).unwrap();
        assert_eq!(plan.writes.len(), 1);
        let w = plan.writes[0];
        assert_eq!(w.card_id, 1026);
        assert_eq!(w.micro, Micro::Stacked { root: 1025, branch: STACK_DIR_UP, index: 0 });
    }

    #[test]
    fn second_stack_appends_next_index() {
        let mz = with_surface(0, WORLD_LAYER);
        let store = Mock::default()
            .with(soul(1024, P))
            .with({ let mut c = loose(1025, 1024, WORLD_LAYER, 1, 1); c.macro_zone = mz; c })
            .with(stacked(1027, 1024, mz, 1025, STACK_DIR_UP, 0)) // existing member at index 0
            .with({ let mut c = loose(1026, 1024, WORLD_LAYER, 2, 2); c.macro_zone = mz; c });
        let plan = plan_place(&store, 1026, Placement::Stack { parent_id: 1025, direction: STACK_DIR_UP }, P, 0).unwrap();
        assert_eq!(plan.writes[0].micro, Micro::Stacked { root: 1025, branch: STACK_DIR_UP, index: 1 });
    }

    #[test]
    fn members_travel_and_reroot_on_stack_move() {
        let mz = with_surface(0, WORLD_LAYER);
        // source 1026 is a loose root with a member 1028 stacked on it.
        let store = Mock::default()
            .with(soul(1024, P))
            .with({ let mut c = loose(1025, 1024, WORLD_LAYER, 1, 1); c.macro_zone = mz; c })
            .with({ let mut c = loose(1026, 1024, WORLD_LAYER, 2, 2); c.macro_zone = mz; c })
            .with(stacked(1028, 1024, mz, 1026, STACK_DIR_UP, 0));
        let plan = plan_place(&store, 1026, Placement::Stack { parent_id: 1025, direction: STACK_DIR_DOWN }, P, 0).unwrap();
        assert_eq!(plan.writes.len(), 2, "source + its member");
        // source becomes member of 1025; its member re-roots onto 1025 keeping branch/index.
        assert_eq!(plan.writes[0].micro, Micro::Stacked { root: 1025, branch: STACK_DIR_DOWN, index: 0 });
        let member = plan.writes.iter().find(|w| w.card_id == 1028).unwrap();
        assert_eq!(member.micro, Micro::Stacked { root: 1025, branch: STACK_DIR_UP, index: 0 });
    }

    #[test]
    fn rejects_cycle_self_and_foreign_owner() {
        let mz = with_surface(0, WORLD_LAYER);
        let store = Mock::default()
            .with(soul(1024, P))
            .with({ let mut c = loose(1025, 1024, WORLD_LAYER, 1, 1); c.macro_zone = mz; c })
            .with(stacked(1026, 1024, mz, 1025, STACK_DIR_UP, 0)) // 1026 is a member of 1025
            .with(soul(2000, 9))
            .with({ let mut c = loose(3000, 2000, WORLD_LAYER, 3, 3); c.macro_zone = mz; c }); // owned by player 9
        // self-stack
        assert!(plan_place(&store, 1025, Placement::Stack { parent_id: 1025, direction: STACK_DIR_UP }, P, 0).is_err());
        // cycle: stacking root 1025 onto its own member 1026
        let err = plan_place(&store, 1025, Placement::Stack { parent_id: 1026, direction: STACK_DIR_UP }, P, 0).unwrap_err();
        assert!(err.contains("cycle"), "{err}");
        // foreign-owned source
        let err = plan_place(&store, 3000, Placement::Stack { parent_id: 1025, direction: STACK_DIR_UP }, P, 0).unwrap_err();
        assert!(err.contains("owned by player 9"), "{err}");
    }

    #[test]
    fn loose_world_placement_lands_at_cell() {
        let store = Mock::default()
            .with(soul(1024, P))
            .with({ let mut c = loose(1026, 1024, WORLD_LAYER, 2, 2); c.macro_zone = with_surface(0, WORLD_LAYER); c });
        let plan = plan_place(
            &store,
            1026,
            Placement::Loose { surface: WORLD_LAYER, macro_zone: with_surface(0, WORLD_LAYER), q: 5, r: 6, x: 0, y: 0 },
            P,
            0,
        )
        .unwrap();
        assert_eq!(plan.writes[0].micro, Micro::Loose { local_q: 5, local_r: 6, x: 0, y: 0 });
    }

    #[test]
    fn loose_into_unowned_container_rejected() {
        // A container macro_zone owned by player 9's soul; caller P may not place.
        let other_soul = 5000u32;
        let inv = with_owner(with_surface(0, INVENTORY_LAYER), other_soul);
        let store = Mock::default()
            .with(soul(2000, 9))
            .with({ let mut c = loose(other_soul, 2000, INVENTORY_LAYER, 0, 0); c.owner_id = 2000; c })
            .with(soul(1024, P))
            .with({ let mut c = loose(1026, 1024, WORLD_LAYER, 2, 2); c.macro_zone = with_surface(0, WORLD_LAYER); c });
        let err = plan_place(
            &store,
            1026,
            Placement::Loose { surface: INVENTORY_LAYER, macro_zone: inv, q: 0, r: 0, x: 0, y: 0 },
            P,
            0,
        )
        .unwrap_err();
        assert!(err.contains("not owned by caller"), "{err}");
    }
}
