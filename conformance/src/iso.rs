// SPDX-License-Identifier: Apache-2.0
//! RDF dataset isomorphism: equal up to renaming of blank nodes.
//!
//! Ground quads (no blank nodes) are compared as sets. Blank nodes are first
//! coloured by their neighbourhood, a few rounds deep, then a bijection
//! between equally coloured blank nodes is searched by backtracking. Blank
//! nodes inside quoted triples (RDF-star) count like any other.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rmlrs::{Quad, Term};

pub fn isomorphic(a: &[Quad], b: &[Quad]) -> bool {
    let a: BTreeSet<Quad> = a.iter().cloned().collect();
    let b: BTreeSet<Quad> = b.iter().cloned().collect();
    if a.len() != b.len() {
        return false;
    }
    let (ga, ba): (Vec<_>, Vec<_>) = a.into_iter().partition(is_ground);
    let (gb, bb): (Vec<_>, Vec<_>) = b.into_iter().partition(is_ground);
    if ga != gb || ba.len() != bb.len() {
        return false;
    }
    if ba.is_empty() {
        return true;
    }
    let ca = colours(&ba);
    let cb = colours(&bb);
    let mut count_a: BTreeMap<&String, usize> = BTreeMap::new();
    let mut count_b: BTreeMap<&String, usize> = BTreeMap::new();
    ca.values()
        .for_each(|c| *count_a.entry(c).or_default() += 1);
    cb.values()
        .for_each(|c| *count_b.entry(c).or_default() += 1);
    if count_a != count_b {
        return false;
    }
    let target: BTreeSet<Quad> = bb.iter().cloned().collect();
    let mut nodes: Vec<&String> = ca.keys().collect();
    nodes.sort_by_key(|n| count_a[&ca[*n]]);
    let mut map = HashMap::new();
    let mut used = BTreeSet::new();
    search(&nodes, 0, &ca, &cb, &mut map, &mut used, &ba, &target)
}

fn is_ground(q: &Quad) -> bool {
    blanks(q).is_empty()
}

/// The blank node labels of a term, quoted triples included.
fn term_blanks<'a>(t: &'a Term, out: &mut Vec<&'a String>) {
    match t {
        Term::BlankNode(b) => out.push(b),
        Term::Triple(t) => {
            term_blanks(&t.subject, out);
            term_blanks(&t.predicate, out);
            term_blanks(&t.object, out);
        }
        _ => {}
    }
}

fn blanks(q: &Quad) -> Vec<&String> {
    let mut out = Vec::new();
    for t in [&q.subject, &q.object].into_iter().chain(q.graph.as_ref()) {
        term_blanks(t, &mut out);
    }
    out
}

/// Colour refinement: each blank node's colour is the sorted multiset of
/// (position, other terms) of the quads it occurs in, iterated.
fn colours(quads: &[Quad]) -> HashMap<String, String> {
    let mut colour: HashMap<String, String> = HashMap::new();
    for q in quads {
        for b in blanks(q) {
            colour.insert(b.clone(), String::new());
        }
    }
    for _ in 0..4 {
        fn show(t: &Term, colour: &HashMap<String, String>) -> String {
            match t {
                Term::BlankNode(b) => format!("_{}", colour[b]),
                Term::Triple(t) => format!(
                    "<< {} {} {} >>",
                    show(&t.subject, colour),
                    show(&t.predicate, colour),
                    show(&t.object, colour)
                ),
                other => other.to_string(),
            }
        }
        let mut next: HashMap<String, Vec<String>> = HashMap::new();
        for q in quads {
            let g = q
                .graph
                .as_ref()
                .map(|g| show(g, &colour))
                .unwrap_or_default();
            let row = |pos: &str| {
                format!(
                    "{pos}|{}|{}|{}|{}",
                    show(&q.subject, &colour),
                    q.predicate,
                    show(&q.object, &colour),
                    g
                )
            };
            for (pos, t) in [
                ("s", Some(&q.subject)),
                ("o", Some(&q.object)),
                ("g", q.graph.as_ref()),
            ] {
                let mut found = Vec::new();
                if let Some(t) = t {
                    term_blanks(t, &mut found);
                }
                for b in found {
                    next.entry(b.clone()).or_default().push(row(pos));
                }
            }
        }
        colour = next
            .into_iter()
            .map(|(k, mut v)| {
                v.sort();
                (k, format!("{:x}", fxhash(&v.join("\n"))))
            })
            .collect();
    }
    colour
}

fn fxhash(s: &str) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in s.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[allow(clippy::too_many_arguments)]
fn search(
    nodes: &[&String],
    i: usize,
    ca: &HashMap<String, String>,
    cb: &HashMap<String, String>,
    map: &mut HashMap<String, String>,
    used: &mut BTreeSet<String>,
    quads: &[Quad],
    target: &BTreeSet<Quad>,
) -> bool {
    if i == nodes.len() {
        return quads.iter().all(|q| target.contains(&rename(q, map)));
    }
    let n = nodes[i];
    let mut candidates: Vec<&String> = cb
        .iter()
        .filter(|(k, c)| **c == ca[n] && !used.contains(*k))
        .map(|(k, _)| k)
        .collect();
    candidates.sort();
    for c in candidates {
        map.insert(n.clone(), c.clone());
        used.insert(c.clone());
        if search(nodes, i + 1, ca, cb, map, used, quads, target) {
            return true;
        }
        used.remove(c);
        map.remove(n);
    }
    false
}

fn rename(q: &Quad, map: &HashMap<String, String>) -> Quad {
    fn r(t: &Term, map: &HashMap<String, String>) -> Term {
        match t {
            Term::BlankNode(b) => Term::BlankNode(map[b].clone()),
            Term::Triple(t) => Term::Triple(Box::new(rmlrs::Triple {
                subject: r(&t.subject, map),
                predicate: r(&t.predicate, map),
                object: r(&t.object, map),
            })),
            other => other.clone(),
        }
    }
    let r = |t: &Term| r(t, map);
    Quad {
        subject: r(&q.subject),
        predicate: q.predicate.clone(),
        object: r(&q.object),
        graph: q.graph.as_ref().map(r),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nq(s: &str) -> Vec<Quad> {
        rmlrs::nquads::parse(s).unwrap()
    }

    #[test]
    fn blank_renaming() {
        let a = nq("_:a <http://e/p> \"1\" .\n_:a <http://e/q> _:b .\n_:b <http://e/p> \"2\" .\n");
        let b = nq("_:x <http://e/p> \"2\" .\n_:y <http://e/q> _:x .\n_:y <http://e/p> \"1\" .\n");
        assert!(isomorphic(&a, &b));
        let c = nq("_:x <http://e/p> \"1\" .\n_:y <http://e/q> _:x .\n_:y <http://e/p> \"2\" .\n");
        assert!(!isomorphic(&a, &c));
    }

    #[test]
    fn plain_and_string_literals_match() {
        let a = nq("<http://e/s> <http://e/p> \"x\" .\n");
        let b =
            nq("<http://e/s> <http://e/p> \"x\"^^<http://www.w3.org/2001/XMLSchema#string> .\n");
        assert!(isomorphic(&a, &b));
    }
}
