// What a story reaches (every function, package, service, endpoint and table) and the project's index made from it
// (`ProjectModel.index`): for each thing, the stories that reach it and where.
//
// A story leaves out much of what it walks: a helper that crosses no boundary folds into one row or is not drawn at
// all. "Which stories use this" is about what runs, not what is drawn, so the project's own functions are recorded
// as the walk reaches them. A use the story does not draw points to the step it is inside.
//
// The index is made from the stories of one reading only and never edited. A story's part of it is its own uses and
// those of the stories it reaches the start of (a step that opens another story, told there): reading one story
// again means replacing its uses and those of the stories that reach it.
use super::{Builder, Ctx};
use crate::model::*;
use crate::source::At;
use std::collections::{BTreeMap, HashMap, HashSet};

/// A use as recorded while walking, with the builder's temporary step ids.
pub struct Reached {
    thing: Thing,
    how: UseKind,
    at: Ref,
    from: Option<String>,
    /// the step that draws it, if one does
    item: Option<String>,
    /// the call whose body it was found in
    inside: Option<String>,
}

/// A finished use of one story, with the thing it reaches.
pub struct Reach {
    pub thing: Thing,
    /// the thing is described by a participant the story draws (preferred over a description made while walking:
    /// an adapter may know it is a component, not a plain function)
    pub drawn_part: bool,
    pub place: Use,
}

impl<'w> Builder<'w> {
    fn thing_of(p: &Participant) -> Thing {
        Thing { id: p.id.clone(), name: p.name.clone(), kind: p.kind, group: p.group, sub: p.sub.clone(), at: p.at.clone(), within: None, uses: vec![] }
    }

    /// The project's own function `decl` is reached at `e` by the code of `ctx.this`. `item` is the step that draws
    /// the call, if there is one.
    pub fn reached(&mut self, decl: At<'w>, e: At<'w>, ctx: &Ctx, item: Option<&str>) {
        let thing = Thing {
            id: self.pid(decl),
            name: self.decl_name(decl),
            kind: PartKind::Function,
            group: ctx.group,
            sub: Some(self.file(decl).rel.clone()),
            at: Some(self.place(decl)),
            within: None,
            uses: vec![],
        };
        let inside = self.anchors.last().cloned();
        let at = self.place(e);
        self.reached.push(Reached { thing, how: UseKind::Calls, at, from: Some(ctx.this.clone()), item: item.map(String::from), inside });
    }

    /// The component `decl` is drawn at `e` by the code of `ctx.this`: what draws what, for the places a person sees.
    pub fn drew(&mut self, decl: At<'w>, e: At<'w>, ctx: &Ctx, item: Option<&str>) {
        let thing = Thing {
            id: self.pid(decl),
            name: self.decl_name(decl),
            kind: PartKind::Component,
            group: ctx.group,
            sub: Some(self.file(decl).rel.clone()),
            at: Some(self.place(decl)),
            within: None,
            uses: vec![],
        };
        let inside = self.anchors.last().cloned();
        let at = self.place(e);
        self.reached.push(Reached { thing, how: UseKind::Draws, at, from: Some(ctx.this.clone()), item: item.map(String::from), inside });
    }

    /// The story starts in this participant.
    pub fn starts(&mut self, part: &str, at: Ref) {
        if let Some(p) = self.parts.iter().find(|p| p.id == part) {
            let thing = Self::thing_of(p);
            self.reached.push(Reached { thing, how: UseKind::Starts, at, from: None, item: None, inside: None });
        }
    }

    /// The story starts in this function, which it does not draw as a participant (a handler on a screen).
    pub fn starts_in(&mut self, decl: At<'w>, group: Group) {
        let thing = Thing {
            id: self.pid(decl),
            name: self.decl_name(decl),
            kind: PartKind::Function,
            group,
            sub: Some(self.file(decl).rel.clone()),
            at: Some(self.place(decl)),
            within: None,
            uses: vec![],
        };
        let at = self.place(decl);
        self.reached.push(Reached { thing, how: UseKind::Starts, at, from: None, item: None, inside: None });
    }

    /// Walks what the step `item` holds (a callee's body): uses found there that no step draws point to it.
    pub fn inside<T>(&mut self, item: &str, walk: impl FnOnce(&mut Self) -> T) -> T {
        let parent = self.anchors.last().cloned();
        self.parents.insert(item.to_string(), parent);
        self.anchors.push(item.to_string());
        let out = walk(self);
        self.anchors.pop();
        out
    }

    /// The story's uses, once its steps have their final ids (`map`: temporary id → final id).
    pub(super) fn uses_of(&mut self, items: &[Item], map: &HashMap<String, String>) -> Vec<Reach> {
        let mut out = vec![];
        // what the drawn steps reach: their participants describe it
        fn walk(list: &[Item], parts: &[Participant], out: &mut Vec<Reach>) {
            for i in list {
                if let Item::Call(c) = i {
                    // a call on the caller's own lifeline (a hook) reaches the callee, recorded while walking
                    if c.from != c.to {
                        let how = match c.effect {
                            Some(Effect::Reads) => UseKind::Reads,
                            Some(Effect::Writes) => UseKind::Writes,
                            Some(Effect::Deletes) => UseKind::Deletes,
                            None => UseKind::Calls,
                        };
                        let place = Use { story: String::new(), step: Some(c.id.clone()), drawn: true, how, at: c.at.clone(), from: Some(c.from.clone()) };
                        let part = parts.iter().find(|p| p.id == c.to);
                        let thing = match part {
                            Some(p) => Builder::thing_of(p),
                            // every drawn step names a participant the story has; if one does not, it still counts
                            None => Thing { id: c.to.clone(), name: c.to.clone(), kind: PartKind::Service, group: Group::Outside, sub: None, at: None, within: None, uses: vec![] },
                        };
                        if let Some(t) = &c.table {
                            let inner = Thing { id: format!("{}/{t}", thing.id), name: t.clone(), sub: Some(thing.name.clone()), at: None, within: Some(thing.id.clone()), uses: vec![], ..thing.clone() };
                            out.push(Reach { thing: inner, drawn_part: true, place: place.clone() });
                        }
                        out.push(Reach { thing, drawn_part: part.is_some(), place });
                    }
                }
                for (_, l) in i.lists() {
                    walk(l, parts, out);
                }
            }
        }
        walk(items, &self.parts, &mut out);
        // what was reached while walking: a step that draws it, else the nearest step it is inside
        for r in std::mem::take(&mut self.reached) {
            let (step, drawn) = match r.item.as_ref().and_then(|t| map.get(t)) {
                Some(s) => (Some(s.clone()), true),
                None => {
                    let mut a = r.inside.clone();
                    loop {
                        match a {
                            Some(x) => match map.get(&x) {
                                Some(s) => break (Some(s.clone()), false),
                                None => a = self.parents.get(&x).cloned().flatten(),
                            },
                            None => break (None, false),
                        }
                    }
                }
            };
            out.push(Reach { thing: r.thing, drawn_part: false, place: Use { story: String::new(), step, drawn, how: r.how, at: r.at, from: r.from } });
        }
        // one use per thing, step, place and kind; the first (a drawn participant's) describes it
        let mut seen = HashSet::new();
        out.retain(|r| seen.insert((r.thing.id.clone(), r.place.step.clone(), r.place.at.clone(), r.place.how)));
        // code walked more than once (a hook's effect, walked inside the hook and again where it runs later): where a
        // step draws it, that is where it is
        let drawn: HashSet<(String, Ref, UseKind)> = out.iter().filter(|r| r.place.drawn).map(|r| (r.thing.id.clone(), r.place.at.clone(), r.place.how)).collect();
        out.retain(|r| r.place.drawn || !drawn.contains(&(r.thing.id.clone(), r.place.at.clone(), r.place.how)));
        // and where a step holds it, a walk that left no step adds nothing
        let held: HashSet<(String, Ref, UseKind)> = out.iter().filter(|r| r.place.step.is_some()).map(|r| (r.thing.id.clone(), r.place.at.clone(), r.place.how)).collect();
        out.retain(|r| r.place.step.is_some() || !held.contains(&(r.thing.id.clone(), r.place.at.clone(), r.place.how)));
        out
    }
}

/// Every step id in a story, in order: taken before and after the steps are named, to map one onto the other.
pub(super) fn step_ids(items: &[Item], out: &mut Vec<String>) {
    for i in items {
        out.push(i.id().to_string());
        for (_, l) in i.lists() {
            step_ids(l, out);
        }
    }
}

/// The index: every thing the stories reach, with its uses in story order. `order` is the stories' final order.
/// A story that reaches the start of another (a step that opens it) reaches what that story reaches, and what the
/// stories that one opens reach in turn: those uses are added to it, not drawn, at that step, unless it reaches the
/// thing itself.
pub fn through_references(stories: &[Story], uses: &mut Vec<Reach>) {
    fn opened(items: &[Item], out: &mut Vec<(String, String)>) {
        for i in items {
            if let Item::Call(c) = i {
                if let Some(o) = &c.opens {
                    out.push((c.id.clone(), format!("open:{o}")));
                }
            }
            for (_, l) in i.lists() {
                opened(l, out);
            }
        }
    }
    let ids: HashSet<&str> = stories.iter().map(|s| s.id.as_str()).collect();
    // each story's references: the step, and the story it opens
    let mut refs: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    for s in stories {
        let mut out = vec![];
        opened(&s.items, &mut out);
        out.retain(|(_, t)| ids.contains(t.as_str()) && *t != s.id);
        if !out.is_empty() {
            refs.insert(s.id.clone(), out);
        }
    }
    let mut by_story: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, u) in uses.iter().enumerate() {
        by_story.entry(u.place.story.clone()).or_default().push(i);
    }
    let mut added = vec![];
    for (story, list) in &refs {
        let mut has: HashSet<String> = by_story.get(story).into_iter().flatten().map(|&i| uses[i].thing.id.clone()).collect();
        for (step, target) in list {
            let mut seen: HashSet<String> = HashSet::from([story.clone()]);
            let mut queue = vec![target.clone()];
            while let Some(t) = queue.pop() {
                if !seen.insert(t.clone()) {
                    continue;
                }
                for &i in by_story.get(&t).into_iter().flatten() {
                    let u = &uses[i];
                    if u.place.how == UseKind::Starts || !has.insert(u.thing.id.clone()) {
                        continue;
                    }
                    let place = Use { story: story.clone(), step: Some(step.clone()), drawn: false, how: u.place.how, at: u.place.at.clone(), from: u.place.from.clone() };
                    added.push(Reach { thing: u.thing.clone(), drawn_part: false, place });
                }
                queue.extend(refs.get(&t).into_iter().flatten().map(|(_, x)| x.clone()));
            }
        }
    }
    uses.extend(added);
}

pub fn index(all: Vec<Reach>, order: &[String]) -> Vec<Thing> {
    let pos: HashMap<&str, usize> = order.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
    let mut by: BTreeMap<String, (Thing, bool)> = BTreeMap::new();
    for r in all {
        let e = by.entry(r.thing.id.clone()).or_insert_with(|| (r.thing.clone(), r.drawn_part));
        if r.drawn_part && !e.1 {
            let uses = std::mem::take(&mut e.0.uses);
            e.0 = Thing { uses, ..r.thing };
            e.1 = true;
        }
        e.0.uses.push(r.place);
    }
    by.into_values()
        .map(|(mut t, _)| {
            // stable: within one story, the order the walk found them
            t.uses.sort_by_key(|u| pos.get(u.story.as_str()).copied().unwrap_or(usize::MAX));
            t
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(id: &str, part: bool, kind: PartKind, story: &str) -> Reach {
        Reach {
            thing: Thing { id: id.into(), name: id.into(), kind, group: Group::Server, sub: None, at: None, within: None, uses: vec![] },
            drawn_part: part,
            place: Use { story: story.into(), step: None, drawn: false, how: UseKind::Calls, at: Ref::default(), from: None },
        }
    }

    #[test]
    fn a_drawn_participant_describes_the_thing_and_uses_follow_the_stories() {
        let all = vec![r("a#f", false, PartKind::Function, "s2"), r("a#f", true, PartKind::Component, "s1"), r("b#g", false, PartKind::Function, "s1")];
        let idx = index(all, &["s1".into(), "s2".into()]);
        assert_eq!(idx.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["a#f", "b#g"]);
        assert_eq!(idx[0].kind, PartKind::Component);
        assert_eq!(idx[0].uses.iter().map(|u| u.story.as_str()).collect::<Vec<_>>(), ["s1", "s2"]);
    }
}
