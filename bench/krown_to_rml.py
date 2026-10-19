# SPDX-License-Identifier: Apache-2.0
"""Rewrites a KROWN-generated mapping (legacy R2RML + old RML vocabulary) to
the current RML vocabulary, so RMLrs can run KROWN scenarios. The engine
itself does not accept the legacy vocabulary (that is M6).

    python krown_to_rml.py mapping.rml.ttl > mapping.ttl

Needs rdflib. Paths under /data/shared/ (KROWN's container layout) become
relative to the working directory.
"""
import sys

from rdflib import Graph, Literal, URIRef

R2RML = "http://www.w3.org/ns/r2rml#"
OLD_RML = "http://semweb.mmlab.be/ns/rml#"
QL = "http://semweb.mmlab.be/ns/ql#"
RML = "http://w3id.org/rml/"
RENAME = {"ReferenceObjectMap": "RefObjectMap", "column": "reference"}


def convert(term):
    if isinstance(term, URIRef):
        s = str(term)
        for ns in (R2RML, OLD_RML, QL):
            if s.startswith(ns):
                local = s[len(ns):]
                return URIRef(RML + RENAME.get(local, local))
    if isinstance(term, Literal) and str(term).startswith("/data/shared/"):
        return Literal(str(term)[len("/data/shared/"):])
    return term


g = Graph().parse(sys.argv[1], format="turtle")
out = Graph()
out.bind("rml", RML)
for s, p, o in g:
    out.add((convert(s), convert(p), convert(o)))
sys.stdout.write(out.serialize(format="turtle"))
