// SPDX-License-Identifier: Apache-2.0
//! Logical views (RML-LV): the iterations of a view are computed from the
//! iterations of the source it is a view on, field by field, then extended
//! by its joins.
//!
//! A view's iterations are computed in full before the first one is
//! mapped: a join needs every iteration of its parent view anyway.

use std::collections::HashMap;
use std::sync::Arc;

use rmlrs_model::{
    Error, Expression, Field, FieldKind, LogicalView, ReferenceFormulation, Result, Source, Term,
    TermType, XSD_INTEGER,
};
use rmlrs_source::{Format, Iteration, JsonPath, NaturalValue, ViewRow};

use crate::{Engine, Value_, cartesian, expr_refs};

/// Values for some keys of a view iteration, by key position.
type Partial = Vec<(usize, NaturalValue)>;

/// What a field's records are read from: an iteration (the source's or an
/// iterable field's), or the text of an expression field's record.
#[derive(Clone, Copy)]
enum Parent<'a> {
    Record(&'a Iteration),
    Text(&'a str),
}

impl<'m> Engine<'m, '_> {
    /// Checks the expressions of a view (and of the views it depends on)
    /// and compiles their references.
    pub(crate) fn compile_view(&mut self, view: &'m LogicalView) -> Result<()> {
        if let Source::View(inner) = &view.view_on.source {
            self.compile_view(inner)?;
        }
        let base = self.format_of(&view.view_on)?;
        self.compile_fields(&view.fields, "", Some(base))?;
        let keys = view_keys(view);
        for j in &view.joins {
            let Source::View(parent) = &j.parent.source else {
                unreachable!("the parser only accepts views as join parents")
            };
            self.compile_view(parent)?;
            let parent_keys = view_keys(parent);
            let (mut child_refs, mut parent_refs) = (Vec::new(), Vec::new());
            for c in &j.conditions {
                expr_refs(&c.child, &mut child_refs);
                expr_refs(&c.parent, &mut parent_refs);
            }
            for f in &j.fields {
                if let FieldKind::Expression(e) = &f.kind {
                    reject_functions(e, &f.name)?;
                    expr_refs(e, &mut parent_refs);
                }
            }
            check_keys(&keys, &child_refs)?;
            check_keys(&parent_keys, &parent_refs)?;
            self.compile_refs(Format::View, child_refs)?;
            self.compile_refs(Format::View, parent_refs)?;
        }
        Ok(())
    }

    /// `format` is the format of the parent records; `None` when the parent
    /// is an expression field, whose records are text.
    fn compile_fields(
        &mut self,
        fields: &'m [Field],
        prefix: &str,
        format: Option<Format>,
    ) -> Result<()> {
        for f in fields {
            let name = absolute(prefix, &f.name);
            match &f.kind {
                FieldKind::Expression(e) => {
                    let format = format.ok_or_else(|| {
                        Error::Mapping(format!(
                            "expression field {name:?} must be nested in an iterable"
                        ))
                    })?;
                    reject_functions(e, &name)?;
                    let mut refs = Vec::new();
                    expr_refs(e, &mut refs);
                    self.compile_refs(format, refs)?;
                    self.compile_fields(&f.fields, &name, None)?;
                }
                FieldKind::Iterable {
                    reference_formulation,
                    iterator,
                } => {
                    let own = reference_formulation
                        .as_ref()
                        .map(field_format)
                        .transpose()?;
                    let records = match (format, own) {
                        (None, Some(own)) => own,
                        (None, None) => {
                            return Err(Error::Mapping(format!(
                                "iterable field {name:?} reads an expression field's records \
                                 and needs rml:referenceFormulation"
                            )));
                        }
                        (Some(parent), Some(own)) if own != parent => {
                            return Err(Error::Mapping(format!(
                                "iterable field {name:?} can only change the reference \
                                 formulation under an expression field"
                            )));
                        }
                        (Some(parent), _) => parent,
                    };
                    if let (Format::Json, Some(it)) = (records, iterator) {
                        JsonPath::parse(it)?;
                    }
                    self.compile_fields(&f.fields, &name, Some(records))?;
                }
            }
        }
        Ok(())
    }

    /// Every iteration of a view, in order.
    pub(crate) fn view_rows(&mut self, view: &'m LogicalView) -> Result<Vec<ViewRow>> {
        let keys = Arc::new(view_keys(view));
        let base: Vec<Iteration> = self.open(&view.view_on)?.collect::<Result<_>>()?;
        // rml:null values of the source apply to the fields read from it.
        let saved = std::mem::replace(&mut self.nulls, &view.view_on.null_values);
        let mut rows = Vec::new();
        let result = (|| {
            for (i, it) in base.iter().enumerate() {
                let mut partials: Vec<Partial> = vec![vec![(position(&keys, "#"), index(i))]];
                for f in &view.fields {
                    let expanded = self.expand(f, "", Parent::Record(it), &keys)?;
                    partials = combine(&partials, &expanded);
                }
                rows.extend(partials.into_iter().map(|p| row(&keys, p)));
            }
            Ok(())
        })();
        self.nulls = &[];
        let result = result.and_then(|()| {
            for j in &view.joins {
                rows = self.join(std::mem::take(&mut rows), j, &keys)?;
            }
            Ok(())
        });
        self.nulls = saved;
        result.map(|()| rows)
    }

    /// The partial iterations one field (with its nested fields) adds to a
    /// parent record: one per record of the field, or a single empty one
    /// (all its keys without value) when the field has no records.
    fn expand(
        &self,
        f: &Field,
        prefix: &str,
        parent: Parent<'_>,
        keys: &[String],
    ) -> Result<Vec<Partial>> {
        let name = absolute(prefix, &f.name);
        let index_key = position(keys, &format!("{name}.#"));
        let mut out = Vec::new();
        match &f.kind {
            FieldKind::Expression(e) => {
                let Parent::Record(it) = parent else {
                    unreachable!("checked by compile_fields")
                };
                for (j, v) in self.record_values(e, it)?.into_iter().enumerate() {
                    let mut partials = vec![vec![
                        (position(keys, &name), v.clone()),
                        (index_key, index(j)),
                    ]];
                    for nested in &f.fields {
                        let e = self.expand(nested, &name, Parent::Text(&v.lexical), keys)?;
                        partials = combine(&partials, &e);
                    }
                    out.extend(partials);
                }
            }
            FieldKind::Iterable {
                reference_formulation,
                iterator,
            } => {
                for (j, rec) in
                    records(parent, reference_formulation.as_ref(), iterator.as_deref())?
                        .iter()
                        .enumerate()
                {
                    let mut partials = vec![vec![(index_key, index(j))]];
                    for nested in &f.fields {
                        let e = self.expand(nested, &name, Parent::Record(rec), keys)?;
                        partials = combine(&partials, &e);
                    }
                    out.extend(partials);
                }
            }
        }
        if out.is_empty() {
            out.push(Vec::new());
        }
        Ok(out)
    }

    /// The records of an expression: its values in their natural form.
    fn record_values(&self, e: &Expression, it: &Iteration) -> Result<Vec<NaturalValue>> {
        Ok(self
            .evaluate(e, it, TermType::Literal)?
            .into_iter()
            .map(|v| match v {
                Value_::Natural(n) => n,
                Value_::Text(t) => NaturalValue::string(t),
                Value_::Constant(Term::Literal(l)) => NaturalValue::string(l.value),
                Value_::Constant(t) => {
                    NaturalValue::string(t.as_iri().map_or_else(|| t.to_string(), str::to_owned))
                }
            })
            .collect())
    }

    /// Extends each iteration with the fields of a join, once per matching
    /// iteration of the parent view.
    fn join(
        &mut self,
        rows: Vec<ViewRow>,
        j: &'m rmlrs_model::ViewJoin,
        keys: &[String],
    ) -> Result<Vec<ViewRow>> {
        let Source::View(parent) = &j.parent.source else {
            unreachable!("the parser only accepts views as join parents")
        };
        let parent_rows = self.view_rows(parent)?;
        let saved = std::mem::take(&mut self.nulls);
        let result = (|| {
            let mut by_key: HashMap<Vec<String>, Vec<usize>> = HashMap::new();
            for (i, p) in parent_rows.iter().enumerate() {
                let it = Iteration::View(p.clone());
                let mut sides = Vec::new();
                for c in &j.conditions {
                    sides.push(self.lexicals(&c.parent, &it)?);
                }
                for k in cartesian(&sides) {
                    by_key.entry(k).or_default().push(i);
                }
            }
            let mut out = Vec::new();
            for r in rows {
                let it = Iteration::View(r.clone());
                let mut sides = Vec::new();
                for c in &j.conditions {
                    sides.push(self.lexicals(&c.child, &it)?);
                }
                let mut matches: Vec<usize> = Vec::new();
                for k in cartesian(&sides) {
                    for &i in by_key.get(&k).into_iter().flatten() {
                        if !matches.contains(&i) {
                            matches.push(i);
                        }
                    }
                }
                if matches.is_empty() {
                    if !j.inner {
                        out.push(r);
                    }
                    continue;
                }
                // A joined field's index counts its records over all matches.
                let mut counts = vec![0; j.fields.len()];
                for i in matches {
                    let it = Iteration::View(parent_rows[i].clone());
                    let mut partials: Vec<Partial> = vec![Vec::new()];
                    for (fi, f) in j.fields.iter().enumerate() {
                        let FieldKind::Expression(e) = &f.kind else {
                            unreachable!("the parser only accepts expression fields in joins")
                        };
                        let mut records: Vec<Partial> = Vec::new();
                        for v in self.record_values(e, &it)? {
                            records.push(vec![
                                (position(keys, &f.name), v),
                                (position(keys, &format!("{}.#", f.name)), index(counts[fi])),
                            ]);
                            counts[fi] += 1;
                        }
                        if records.is_empty() {
                            records.push(Vec::new());
                        }
                        partials = combine(&partials, &records);
                    }
                    for p in partials {
                        let mut extended = r.clone();
                        for (k, v) in p {
                            extended.values[k] = Some(v);
                        }
                        out.push(extended);
                    }
                }
            }
            Ok(out)
        })();
        self.nulls = saved;
        result
    }
}

/// The referenceable keys of a view: `#`, every expression field and every
/// index key, joined fields included. Record keys of iterable fields are
/// not referenceable.
pub(crate) fn view_keys(view: &LogicalView) -> Vec<String> {
    fn walk(fields: &[Field], prefix: &str, out: &mut Vec<String>) {
        for f in fields {
            let name = absolute(prefix, &f.name);
            if matches!(f.kind, FieldKind::Expression(_)) {
                out.push(name.clone());
            }
            out.push(format!("{name}.#"));
            walk(&f.fields, &name, out);
        }
    }
    let mut keys = vec!["#".to_owned()];
    walk(&view.fields, "", &mut keys);
    for j in &view.joins {
        walk(&j.fields, "", &mut keys);
    }
    keys
}

/// Rejects references to keys a view does not have, or cannot be referenced.
pub(crate) fn check_keys(keys: &[String], refs: &[String]) -> Result<()> {
    match refs.iter().find(|r| !keys.contains(r)) {
        Some(r) => Err(Error::Mapping(format!(
            "the logical view has no referenceable key {r:?}"
        ))),
        None => Ok(()),
    }
}

fn reject_functions(e: &Expression, field: &str) -> Result<()> {
    match e {
        Expression::Function(_) => Err(Error::Unsupported(format!(
            "function execution in logical view field {field:?}"
        ))),
        _ => Ok(()),
    }
}

/// The records an iterable field selects from its parent.
fn records(
    parent: Parent<'_>,
    rf: Option<&ReferenceFormulation>,
    iterator: Option<&str>,
) -> Result<Vec<Iteration>> {
    match parent {
        Parent::Text(text) => {
            let format = field_format(rf.expect("checked by compile_fields"))?;
            format
                .open(Box::new(text.as_bytes()), iterator, &[])?
                .collect()
        }
        Parent::Record(Iteration::Json(v)) => Ok(JsonPath::parse(iterator.unwrap_or("$"))?
            .select(v)
            .into_iter()
            .cloned()
            .map(Iteration::Json)
            .collect()),
        // A CSV row (or database row) has nothing nested to iterate over.
        Parent::Record(it @ (Iteration::Csv(_) | Iteration::Sql(_))) if iterator.is_none() => {
            Ok(vec![it.clone()])
        }
        Parent::Record(_) => Err(Error::Unsupported(
            "iterable field over this kind of record".into(),
        )),
    }
}

fn field_format(rf: &ReferenceFormulation) -> Result<Format> {
    match rf {
        ReferenceFormulation::JsonPath => Ok(Format::Json),
        ReferenceFormulation::Csv => Ok(Format::Csv),
        other => Err(Error::Unsupported(format!(
            "reference formulation {other:?} in an iterable field"
        ))),
    }
}

fn absolute(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

fn position(keys: &[String], key: &str) -> usize {
    keys.iter()
        .position(|k| k == key)
        .expect("view keys cover every field")
}

fn index(i: usize) -> NaturalValue {
    NaturalValue {
        lexical: i.to_string(),
        datatype: XSD_INTEGER,
    }
}

/// Every combination of one partial iteration from each side.
fn combine(left: &[Partial], right: &[Partial]) -> Vec<Partial> {
    let mut out = Vec::with_capacity(left.len() * right.len());
    for l in left {
        for r in right {
            let mut p = l.clone();
            p.extend(r.iter().cloned());
            out.push(p);
        }
    }
    out
}

fn row(keys: &Arc<Vec<String>>, partial: Partial) -> ViewRow {
    let mut values = vec![None; keys.len()];
    for (k, v) in partial {
        values[k] = Some(v);
    }
    ViewRow {
        keys: keys.clone(),
        values,
    }
}
