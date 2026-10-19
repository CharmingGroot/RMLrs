// SPDX-License-Identifier: Apache-2.0
//! Round trip of the JSON-LD and RDF/XML targets: a mapping writes the same
//! quads to an N-Quads, a JSON-LD and an RDF/XML (UTF-16) target, and the
//! readers of this crate must get the same dataset back from each. RDF/XML
//! has no named graphs, so it only gets the default graph.

use std::fs;

use crate::{iso, read_rdf};

const MAPPING: &str = r#"
@prefix rml: <http://w3id.org/rml/> .
@prefix formats: <http://www.w3.org/ns/formats/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.com/> .
@base <http://example.com/rules/> .

<#Source> a rml:LogicalSource;
  rml:source [ a rml:FilePath; rml:root rml:MappingDirectory; rml:path "people.json" ];
  rml:referenceFormulation rml:JSONPath;
  rml:iterator "$[*]" .

<#People> a rml:TriplesMap;
  rml:logicalSource <#Source>;
  rml:subjectMap [ rml:template "http://example.com/person/{$.id}";
                   rml:logicalTarget <#nq>, <#ld>, <#xml> ];
  rml:predicateObjectMap [ rml:predicate ex:name; rml:objectMap [ rml:reference "$.name" ] ];
  rml:predicateObjectMap [ rml:predicate ex:label;
    rml:objectMap [ rml:reference "$.name"; rml:language "en-GB" ] ];
  rml:predicateObjectMap [ rml:predicate ex:age;
    rml:objectMap [ rml:reference "$.age"; rml:datatype xsd:integer ] ];
  rml:predicateObjectMap [ rml:predicate ex:friend;
    rml:objectMap [ rml:template "{$.name}"; rml:termType rml:BlankNode ] ];
  rml:predicateObjectMap [ rml:predicate ex:home;
    rml:objectMap [ rml:template "http://example.com/place?q={$.name}&x=1" ] ] .

<#Friends> a rml:TriplesMap;
  rml:logicalSource <#Source>;
  rml:subjectMap [ rml:template "{$.name}"; rml:termType rml:BlankNode;
                   rml:logicalTarget <#nq>, <#xml>, <#ld> ];
  rml:predicateObjectMap [ rml:predicate ex:seen; rml:object "yes" ] .

<#Graphs> a rml:TriplesMap;
  rml:logicalSource <#Source>;
  rml:subjectMap [ rml:template "http://example.com/person/{$.id}";
                   rml:graphMap [ rml:template "http://example.com/graph/{$.id}" ];
                   rml:logicalTarget <#nq>, <#ld> ];
  rml:predicateObjectMap [ rml:predicate ex:inGraph; rml:object "yes"@en ] .

<#nq> a rml:LogicalTarget; rml:serialization formats:N-Quads;
  rml:target [ a rml:Target, rml:FilePath; rml:root rml:MappingDirectory; rml:path "out.nq" ] .
<#ld> a rml:LogicalTarget; rml:serialization formats:JSON-LD;
  rml:target [ a rml:Target, rml:FilePath; rml:root rml:MappingDirectory; rml:path "out.jsonld" ] .
<#xml> a rml:LogicalTarget; rml:serialization formats:RDF_XML; rml:encoding rml:UTF-16;
  rml:target [ a rml:Target, rml:FilePath; rml:root rml:MappingDirectory; rml:path "out.rdfxml" ] .
"#;

const PEOPLE: &str = r#"[
  { "id": 0, "name": "Ann & <Bob> \"q\" 'a'\r\n\tend", "age": 33 },
  { "id": 1, "name": "  Zoë ]]> é  ", "age": 34 }
]"#;

#[test]
fn json_ld_and_rdf_xml_read_back_as_written() {
    let dir = std::env::temp_dir().join(format!("rmlrs-round-trip-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("people.json"), PEOPLE).unwrap();
    let mapping = rmlrs::parse_mapping(MAPPING, None).unwrap();
    let options = rmlrs::Options {
        base_iri: "http://example.com/".into(),
        mapping_dir: dir.clone(),
        working_dir: dir.clone(),
        ..Default::default()
    };
    rmlrs::run_with_targets(&mapping, &options, &mut |_| Ok(())).unwrap();

    let nq = read_rdf(&dir.join("out.nq")).unwrap();
    let ld = read_rdf(&dir.join("out.jsonld")).unwrap();
    let xml = read_rdf(&dir.join("out.rdfxml")).unwrap();
    let raw_xml = fs::read(dir.join("out.rdfxml")).unwrap();
    fs::remove_dir_all(&dir).ok();

    assert_eq!(nq.len(), 14, "{}", rmlrs::to_nquads(&nq));
    assert!(nq.iter().any(|q| q.graph.is_some()));
    assert!(
        iso::isomorphic(&ld, &nq),
        "JSON-LD:\n{}",
        rmlrs::to_nquads(&ld)
    );
    let default_graph: Vec<_> = nq.into_iter().filter(|q| q.graph.is_none()).collect();
    assert!(
        iso::isomorphic(&xml, &default_graph),
        "RDF/XML:\n{}",
        rmlrs::to_nquads(&xml)
    );
    assert_eq!(raw_xml[..2], [0xFF, 0xFE], "UTF-16LE byte order mark");
}
