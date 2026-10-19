# SPDX-License-Identifier: Apache-2.0
"""Generates the CSV benchmark input and its mapping: gen_people.py <dir> <rows>"""
import pathlib
import sys

out, n = pathlib.Path(sys.argv[1]), int(sys.argv[2])
out.mkdir(parents=True, exist_ok=True)
with open(out / f"people_{n}.csv", "w") as f:
    f.write("id,first,last,email,city,age\n")
    for i in range(n):
        f.write(f"{i},First{i % 5000},Last{i % 7919},user{i}@example.org,City {i % 300},{20 + i % 60}\n")
(out / f"mapping_{n}.ttl").write_text(f'''@prefix rml: <http://w3id.org/rml/> .
@prefix ex: <http://example.com/ns#> .
<#People> a rml:TriplesMap ;
  rml:logicalSource [ rml:source "people_{n}.csv" ; rml:referenceFormulation rml:CSV ] ;
  rml:subjectMap [ rml:template "http://example.com/person/{{id}}" ; rml:class ex:Person ] ;
  rml:predicateObjectMap [ rml:predicate ex:first ; rml:objectMap [ rml:reference "first" ] ] ;
  rml:predicateObjectMap [ rml:predicate ex:last ; rml:objectMap [ rml:reference "last" ] ] ;
  rml:predicateObjectMap [ rml:predicate ex:email ; rml:objectMap [ rml:reference "email" ] ] ;
  rml:predicateObjectMap [ rml:predicate ex:city ; rml:objectMap [ rml:template "http://example.com/city/{{city}}" ] ] ;
  rml:predicateObjectMap [ rml:predicate ex:age ; rml:objectMap [ rml:reference "age" ] ] .
''')
