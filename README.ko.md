# RMLrs

[English](README.md) | [한국어](README.ko.md)

[![CI](https://github.com/CharmingGroot/RMLrs/actions/workflows/ci.yml/badge.svg)](https://github.com/CharmingGroot/RMLrs/actions/workflows/ci.yml)
[![License: Apache-2.0](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![MSRV 1.88](https://img.shields.io/badge/rustc-1.88+-orange.svg)](Cargo.toml)

**Rust로 작성한 RML 엔진입니다.** RMLrs는 [RML](https://w3id.org/rml/portal/)(RDF Mapping Language) 명세에 따라 JSON, CSV, XML 및 관계형 데이터를 RDF로 변환합니다. [R2RML](https://www.w3.org/TR/r2rml/) 매핑과 이전 RML 어휘로 작성된 매핑도 실행합니다.

명령줄 도구와 Rust 라이브러리로 사용할 수 있습니다. Python 및 WebAssembly 모듈은 메모리에서 전달한 JSON·CSV 문서를 같은 라이브러리로 처리하며, RML-Core 테스트 76개에서 명령줄 도구와 동일한 결과를 냅니다.

## 프리릴리스 상태

**RMLrs는 프리릴리스 프로젝트입니다.** 첫 GitHub 프리릴리스는 [`v0.1.0-alpha.1`](https://github.com/CharmingGroot/RMLrs/releases/tag/v0.1.0-alpha.1)입니다. Cargo 워크스페이스 버전은 아직 `0.0.0`이며 crate, wheel, WebAssembly 패키지 및 바이너리는 배포되지 않았습니다. 아래 안내에 따라 소스에서 빌드하세요. 안정판 전에는 공개 API와 지원 기능이 바뀔 수 있습니다. 이 문서의 명세 적합성 및 성능 수치는 저장소에 포함된 실행기와 벤치마크로 측정했습니다.

## 목차

- [프리릴리스 상태](#프리릴리스-상태)
- [무엇을 하나요?](#무엇을-하나요)
- [적합한 용도](#적합한-용도)
- [명세 적합성](#명세-적합성)
- [기능](#기능)
- [설치](#설치)
- [사용법](#사용법)
- [성능](#성능)
- [문서](#문서)
- [기여하기](#기여하기)
- [라이선스](#라이선스)

## 무엇을 하나요?

RML 매핑에는 입력 레코드를 RDF 트리플로 변환하는 규칙을 적습니다. RMLrs는 입력을 읽고 그 규칙에 따라 트리플을 출력합니다.

`people.json`:

```json
[
  { "id": 1, "name": "Alice", "knows": [2] },
  { "id": 2, "name": "Bob",   "knows": [] }
]
```

`mapping.ttl`:

```turtle
@prefix rml:  <http://w3id.org/rml/> .
@prefix foaf: <http://xmlns.com/foaf/0.1/> .

<#Person> a rml:TriplesMap ;
  rml:logicalSource [
    rml:source [ a rml:RelativePathSource ; rml:root rml:MappingDirectory ; rml:path "people.json" ] ;
    rml:referenceFormulation rml:JSONPath ;
    rml:iterator "$[*]"
  ] ;
  rml:subjectMap [ rml:template "http://example.com/person/{$.id}" ; rml:class foaf:Person ] ;
  rml:predicateObjectMap [ rml:predicate foaf:name ; rml:objectMap [ rml:reference "$.name" ] ] ;
  rml:predicateObjectMap [ rml:predicate foaf:knows ;
                           rml:objectMap [ rml:template "http://example.com/person/{$.knows[*]}" ] ] .
```

```console
$ rmlrs run mapping.ttl
<http://example.com/person/1> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://example.com/person/1> <http://xmlns.com/foaf/0.1/name> "Alice" .
<http://example.com/person/1> <http://xmlns.com/foaf/0.1/knows> <http://example.com/person/2> .
<http://example.com/person/2> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://xmlns.com/foaf/0.1/Person> .
<http://example.com/person/2> <http://xmlns.com/foaf/0.1/name> "Bob" .
```

같은 매핑 언어로 여러 입력을 연결할 수 있습니다. JSON API, SQL 테이블, CSV 파일이 같은 대상을 설명한다면 결과를 하나의 RDF 노드로 합칠 수 있습니다.

## 적합한 용도

RMLrs는 단일 컴퓨터에서 실행하는 일괄 매핑이나 Rust 라이브러리·명령줄·Python·WebAssembly에서 같은 RML 구현을 사용하려는 애플리케이션에 적합합니다. 파일, 데이터베이스 및 HTTP(S) 입력을 처리합니다. Kafka/MQTT의 연속 스트림 처리기나 SPARQL 엔진은 아닙니다. 입력이 크다면 [메모리 사용 특성](docs/architecture.ko.md#메모리와-스트리밍)을 확인하세요. 현재 프리릴리스는 소스 빌드가 필요합니다.

## 명세 적합성

아래는 저장소의 [`rmlrs-conformance`](conformance/)로 공식 테스트 스위트를 실행한 결과입니다. 비 SQL 테스트는 변경 때마다, SQL 테스트는 PostgreSQL·MySQL·SQL Server 컨테이너를 사용하여 야간에 실행합니다. 통과하지 못한 사례는 미구현이거나 [`known-issues.csv`](conformance/known-issues.csv)에 이유가 기록되어 있습니다. 아래 수치에는 야간 실행의 데이터베이스 사례가 포함됩니다. `--databases` 없이 실행하면 이 사례들은 실행되지 않습니다.

| 테스트 스위트 | 통과 | 비고 |
|---|---:|---|
| [RML-Core](https://w3id.org/rml/core/spec) | **76 / 76** | |
| [RML-IO](https://w3id.org/rml/io/spec) | 70 / 73 | RDF 파일에 대한 SPARQL 질의는 미지원(내장 SPARQL 엔진 없음); 실행기가 아직 설정할 수 없는 DB 사례 1개; 목록에 기록된 사례 1개 |
| [RML-IO-Registry](https://github.com/kg-construct/rml-io-registry) | 79 / 102 | SQL 63 / 77(나머지 14개는 기록된 테스트 스위트 문제); 그 외 입력 16 / 25(기록된 사례 6개, Kafka/MQTT 스트림 미지원, 공개 SPARQL 엔드포인트가 필요한 사례 1개) |
| [RML-FNML](https://w3id.org/rml/fnml/spec) | 19 / 20 | 나머지 1개는 기록된 테스트 스위트 문제 |
| [RML-CC](https://w3id.org/rml/cc/spec) (컬렉션과 컨테이너) | **35 / 35** | |
| [RML-LV](https://w3id.org/rml/lv/spec) (논리 뷰) | **41 / 41** | |
| [RML-star](https://w3id.org/rml/star/spec) (RDF-star) | 10 / 18 | 나머지 8개는 입력의 불필요한 한 줄을 제거하면 통과(기록됨) |
| [W3C R2RML](https://www.w3.org/2001/sw/rdb2rdf/test-cases/) (PostgreSQL) | 61 / 62 | 나머지 1개는 기록됨 |
| [이전 RML 테스트](https://github.com/RMLio/rml-test-cases) | 260 / 323 | 24개에는 실행기가 시작하지 않는 SPARQL 서버가 필요; 39개는 기록됨(대부분 이 스위트와 W3C R2RML 스위트가 서로 반대 결과를 기대하는 사례) |

통과·실패 판정 기준과 직접 실행하는 방법은 [docs/conformance.md](docs/conformance.md)에 있습니다.

## 기능

**입력**

- JSON/JSONPath, CSV, XML/XPath 3.1
- 구분자, 인용, 헤더, 건너뛸 행, 공백 제거, null 값, 인코딩 등의 방언을 포함한 CSVW 테이블 설명
- PostgreSQL, MySQL, SQL Server(`d2rq:Database`)의 테이블과 질의, SQL 값의 자연스러운 RDF 자료형. PostgreSQL·MySQL 행은 점진적으로 가져오고 SQL Server 결과는 버퍼에 저장
- 매핑 파일 또는 작업 디렉터리를 기준으로 한 파일, 또는 호출자가 메모리로 전달한 문서
- DCAT·Web of Things로 기술한 HTTP(S) 다운로드와 CSV·TSV·JSON 결과를 반환하는 SPARQL 엔드포인트(`http` 기능, CLI에서 기본 활성화)
- UTF-8 및 UTF-16 입력
- GZip, Zip, Tar+GZip, Tar+Xz 압축 입력

**매핑 언어**

- 조인, 명명 그래프, 언어·자료형 매핑, 빈 노드, 기준 IRI를 포함한 RML-Core
- 테스트 스위트에서 쓰는 내장 함수 및 사용자 정의 함수 API를 포함한 RML-FNML 함수 실행과 조건
- 여러 레코드에 걸쳐 확장되는 명명 컬렉션을 포함한 RML-CC 컬렉션·컨테이너(`rdf:List`, `rdf:Bag`, `rdf:Seq`, `rdf:Alt`)
- 필드, 중첩 필드, 인덱스 키, 왼쪽·내부 조인, 필드 내부 참조 방식 변경을 지원하는 RML-LV 논리 뷰
- RML-star 인용 트리플과 단언·비단언 트리플 맵
- `--db-url`로 데이터베이스를 지정하는 R2RML 매핑과 이전 RML 어휘(`semweb.mmlab.be/ns/rml#`, `ql:`) 매핑

**출력**

- 표준 출력 또는 파일로 내보내는 N-Quads. 인용 트리플은 `<< s p o >>` 형태
- UTF-8·UTF-16 및 선택적 압축을 지원하는 RML-IO 논리 대상: N-Quads, N-Triples, Turtle, N3, RDF/JSON, JSON-LD, RDF/XML 파일
- 중복 쿼드 제거

**아직 지원하지 않음:** Kafka·MQTT 스트림 입력, RDF 파일에 대한 SPARQL 질의(내장 SPARQL 엔진 없음), CSVW 테이블 스키마, UTF-8·UTF-16 이외의 입력 인코딩, RDF/JSON·JSON-LD·RDF/XML 대상의 인용 트리플. RML 명세 밖의 매핑 기능은 범위에 포함하지 않습니다([PHILOSOPHY.md](PHILOSOPHY.md)).

## 설치

사전 빌드 패키지는 아직 배포되지 않았습니다. Rust 1.88 이상이 필요합니다.

```bash
cargo install --git https://github.com/CharmingGroot/RMLrs --tag v0.1.0-alpha.1 rmlrs-cli
```

모든 입력 형식과 데이터베이스 드라이버를 포함한 `rmlrs` 명령을 설치합니다. Python·WebAssembly 빌드는 [bindings/python](bindings/python/README.md), [bindings/wasm](bindings/wasm/README.md)을 참고하세요.

## 사용법

**명령줄**

```bash
rmlrs run mapping.ttl -o out.nq --base-iri http://example.com/

# R2RML: 매핑은 테이블을 지정하고, 데이터베이스는 여기서 지정합니다.
RMLRS_DB_PASSWORD=secret rmlrs run r2rml.ttl \
  --db-url jdbc:postgresql://localhost:5432/mydb --db-user me
```

**Rust**

```rust
let quads = rmlrs::run_file("mapping.ttl", "http://example.com/")?;
print!("{}", rmlrs::to_nquads(&quads));
```

`rmlrs::run_to`와 `rmlrs::run_with_targets`는 쿼드가 생성되는 즉시 호출자에게 전달하므로 전체 출력을 메모리에 보관할 필요가 없습니다.

**Python**

```python
import rmlrs
nquads = rmlrs.materialize(mapping_ttl, base_iri="http://example.com/",
                           sources={"people.json": json_text})
```

**JavaScript (WebAssembly)**

```js
import init, { materialize } from "./pkg/rmlrs_wasm.js";
await init();
const nquads = materialize(mappingTtl, { "people.json": jsonText }, "http://example.com/");
```

메모리 입력, 데이터베이스, 논리 대상, 사용자 정의 함수, Cargo 기능에 관한 자세한 내용은 [docs/usage.md](docs/usage.md)에 있습니다.

## 성능

RMLrs는 CSV 및 PostgreSQL·MySQL 행을 점진적으로 읽고 쿼드가 생성되는 즉시 출력합니다. JSON 문서와 SQL Server 질의 결과는 메모리에 보관합니다. XML 입력은 전체를 읽은 다음, 조건에 맞는 지역 반복 구간을 나누어 파싱합니다. 조인, 논리 뷰 및 일부 출력 형식에도 상태가 필요합니다. 중복 제거에는 서로 다른 쿼드마다 16바이트 지문을 보관합니다. 입력·출력별 한계는 [아키텍처 문서](docs/architecture.ko.md#메모리와-스트리밍)를 참고하세요.

[KROWN](https://github.com/kg-construct/KROWN)의 실행 프레임워크로 Morph-KGC 2.10.0(프로세스 4개)과 비교했습니다. 각 시나리오를 세 번 실행했으며 모든 실행에서 같은 출력이 나왔습니다(Apple Silicon Mac, CPU 10개·메모리 8GB의 Docker VM).

| 시나리오 | RMLrs | Morph-KGC |
|---|---:|---:|
| PostgreSQL, 10만 행 × 20열 | 2.27–2.57초 | 5.07–6.24초 |
| PostgreSQL, 50% 조인 | 1.13–1.56초 | 2.47–2.85초 |
| MySQL, 10만 행 × 20열 | 2.19–2.51초 | 5.56–6.22초 |
| MySQL, 50% 조인 | 0.82–1.14초 | 2.58–3.68초 |

이 환경은 KROWN 논문의 실험 환경과 다릅니다. 측정 방법, 지금까지 측정한 전체 시나리오 및 재현 방법은 [docs/benchmarks.md](docs/benchmarks.md)에 있습니다.

## 문서

- [docs/usage.md](docs/usage.md): 명령줄, 라이브러리, 바인딩, 데이터베이스, 출력 대상, 함수
- [docs/conformance.md](docs/conformance.md): 테스트 스위트, 실행기, 알려진 문제
- [docs/benchmarks.md](docs/benchmarks.md): 성능 측정 방법과 결과
- [docs/architecture.ko.md](docs/architecture.ko.md): 크레이트 구성과 매핑 실행 과정
- [docs/provenance.md](docs/provenance.md): 크레이트별 구현 명세와 의존 근거
- [PHILOSOPHY.md](PHILOSOPHY.md): 범위와 새 기능을 판단하는 원칙
- [ROADMAP.md](ROADMAP.md): 마일스톤과 후속 작업
- [docs/adr](docs/adr): 아키텍처 결정 기록
- [CHANGELOG.md](CHANGELOG.md)

## 기여하기

버그 보고, 테스트 사례, 풀 리퀘스트를 환영합니다. 먼저 [기여 가이드](CONTRIBUTING.ko.md)를 읽어주세요. 변경에는 테스트가 필요하고, 명세 적합성 기준선이 유지되어야 하며, 커밋에 DCO 서명이 필요합니다. [행동 강령](CODE_OF_CONDUCT.ko.md)을 준수해 주세요. 보안 문제는 [보안 정책](SECURITY.ko.md)에 안내된 비공개 채널로 신고해 주세요.

## 라이선스

[Apache License 2.0](LICENSE)을 적용합니다. 타사 코드와 테스트에 사용한 테스트 스위트는 [NOTICE](NOTICE)에 기록되어 있습니다.

## 감사의 말

[W3C Knowledge Graph Construction Community Group](https://www.w3.org/community/kg-construct/)의 테스트 스위트, [KROWN](https://github.com/kg-construct/KROWN) 벤치마크, [RML.io](https://rml.io/) 테스트 사례, W3C RDB2RDF Working Group의 R2RML 테스트 사례 덕분에 위 수치를 측정할 수 있었습니다.
