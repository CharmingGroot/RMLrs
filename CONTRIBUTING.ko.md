# RMLrs에 기여하기

[English](CONTRIBUTING.md) | [한국어](CONTRIBUTING.ko.md)

관심을 가져주셔서 감사합니다. 버그 보고, 실패하는 매핑, 테스트 사례, 문서 및 코드 기여를 환영합니다. 이 문서는 변경 사항이 RMLrs에 반영되는 과정을 설명합니다.

[행동 강령](CODE_OF_CONDUCT.ko.md)을 준수해 주세요. 보안 문제는 공개 이슈 대신 [보안 정책](SECURITY.ko.md)에 안내된 비공개 채널로 신고해 주세요.

## 프로젝트 범위

RMLrs는 RML 명세에 정의된 기능을 구현합니다([PHILOSOPHY.md](PHILOSOPHY.md)). 유용하더라도 RML 명세 모듈에 없는 기능은 이 저장소의 범위에 포함되지 않습니다. 그런 기능은 먼저 [KG-Construct Community Group](https://www.w3.org/community/kg-construct/)에 제안해 주세요. 명세 범위 안에서는 누락된 기능, 잘못된 출력, 발생해야 하거나 발생하면 안 되는 오류, 속도 및 메모리 사용량 모두 기여 대상입니다.

작은 수정을 넘는 변경이라면 먼저 이슈를 열어 접근 방법을 논의해 주세요.

## 버그 보고

재현에 필요한 내용을 포함해 주세요.

- 가능한 한 작게 줄인 매핑과 입력 데이터
- 실행한 명령 또는 코드와 RMLrs 커밋
- 기대한 출력, 실제 출력, 발생한 오류

다른 RML 엔진과 결과가 다르다는 보고도 유용합니다. 비교한 엔진과 버전을 적어주세요.

## 개발 환경

- Rust 1.88 이상(`rustup`), `rustfmt`, `clippy`
- SQL 명세 적합성 사례와 데이터베이스 테스트를 위한 Docker
- Git 서브모듈로 관리하는 테스트 스위트

```bash
git clone --recurse-submodules https://github.com/CharmingGroot/RMLrs
# 이미 복제한 저장소라면:
git submodule update --init
```

## 빌드와 테스트

코드를 제출하기 전에 아래 검사를 실행하세요. CI도 포맷, 린트, 테스트, 라이선스 및 명세 적합성 검사를 실행합니다.

```bash
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all
cargo deny check                      # 라이선스와 보안 권고 확인(cargo install cargo-deny)
```

명세 적합성 실행기는 공식 테스트 스위트를 실행합니다([docs/conformance.md](docs/conformance.md)).

```bash
cargo run --release -p rmlrs-conformance -- --check conformance/baseline.csv
cargo run --release -p rmlrs-conformance -- --databases --check conformance/baseline.csv   # SQL 사례 포함
```

실제 데이터베이스가 필요한 테스트에는 `#[ignore]`가 붙어 있으며 환경 변수에서 서버 주소를 읽습니다.

```bash
docker run -d --rm -p 127.0.0.1:55432:5432 -e POSTGRES_HOST_AUTH_METHOD=trust postgres:16-alpine
RMLRS_TEST_POSTGRES=jdbc:postgresql://127.0.0.1:55432/postgres \
  cargo test -p rmlrs-source --features postgres -- --ignored
```

MySQL에는 `RMLRS_TEST_MYSQL`을 같은 방식으로 사용할 수 있으며 사용자 이름과 비밀번호는 `root`입니다.

## 풀 리퀘스트

- **풀 리퀘스트 하나에 주제 하나만 담아주세요.** 제목은 `area: summary` 형식으로 작성합니다. 예: `source/json: support bracket-quoted names`, `engine: hash join for referencing object maps`. `area`에는 크레이트 또는 모듈 이름(`engine`, `parser`, `source/sql`, `cc`, `lv`, `conformance`, `docs` 등)을 사용합니다.
- **변경에는 테스트를 포함하세요.** 통과하도록 만든 명세 테스트 사례의 ID를 적거나, 공식 사례가 없다면 단위 테스트를 추가하세요. 수정 사항을 되돌리면 해당 테스트가 실패하는지 로컬에서 확인하세요. 버그 수정에는 회귀 테스트를, 새 기능에는 정상 동작과 관련 오류에 대한 테스트를 포함하세요. 자동화된 테스트가 현실적으로 어렵다면 풀 리퀘스트에 이유를 설명하세요. 문서만 수정하는 경우 새 테스트가 필요하지 않습니다.
- **기존 통과 사례를 깨뜨리지 마세요.** PR CI는 데이터베이스를 사용하지 않는 명세 테스트를 `conformance/baseline.csv`와 비교합니다. 통과하던 검사 대상 사례가 실패하면 CI도 실패합니다. SQL 사례는 Docker를 사용해 야간에 실행합니다. 새로 통과하는 사례가 생기면 같은 PR에서 기준선을 갱신하고 바뀐 사례 ID를 설명하세요.

  ```bash
  cargo run --release -p rmlrs-conformance -- --databases --write-baseline conformance/baseline.csv
  ```

- **필수 검사를 통과해야 합니다.** 포맷, 경고를 허용하지 않는 clippy, 테스트, `cargo deny`를 확인합니다. 병합 전 PR의 `check`, `msrv`, `dco`, `deny`, `bindings` CI 작업이 모두 통과해야 합니다. 데이터베이스 명세 적합성은 야간 워크플로에서 실행하며 PR 필수 검사는 아닙니다.
- **사용자에게 보이는 동작이 바뀌면 문서도 갱신하세요.** [docs/usage.md](docs/usage.md), README 표 또는 [CHANGELOG.md](CHANGELOG.md)의 `Unreleased` 항목을 갱신합니다. 해당 문서의 한국어판이 있다면 함께 갱신하세요.

### 테스트 사례가 잘못된 것 같다면

테스트 스위트에도 오류가 있을 수 있습니다. 기대 결과가 잘못되었다고 판단되더라도 RMLrs에 그 사례만을 위한 예외 처리를 추가하지 마세요. 명세 조항, 모순되는 다른 사례, 잘못된 입력 파일 등 다른 사람이 확인할 수 있는 근거를 [`conformance/known-issues.csv`](conformance/known-issues.csv)에 기록하고, 가능하면 상위 프로젝트에도 보고해 주세요.

## 커밋

모든 커밋에는 [Developer Certificate of Origin](https://developercertificate.org/)(DCO)에 따른 서명이 필요합니다. 이 서명은 직접 작성했거나 제출할 권한이 있음을 확인합니다.

```bash
git commit -s
```

커밋 제목은 PR과 같은 `area: summary` 형식으로 작성하고, 이유가 명확하지 않다면 본문에 설명을 덧붙이세요.

## 코드

- 모든 소스 파일은 `// SPDX-License-Identifier: Apache-2.0`으로 시작합니다.
- 알맞은 오류 종류를 반환하세요. 잘못된 매핑은 `Mapping`, 유효한 RDF를 만들 수 없는 데이터는 `Data`, 읽을 수 없는 입력은 `Source`, 미구현 기능은 `Unsupported`입니다. 명세 적합성 실행기는 이 구분에 의존합니다.
- 표준 라이브러리와 기존 의존성을 우선 사용하세요. 새 의존성이 필요하다면 PR에 이유를 적고 `cargo deny`를 통과해야 합니다.
- 다른 프로젝트에서 가져온 코드에는 호환되는 라이선스와 [NOTICE](NOTICE) 항목이 필요합니다.
- 큰 설계 결정은 [docs/adr](docs/adr)에 ADR로 기록합니다.

## 저장소 공통 지침

이 문서와 공개 문서, CI가 편집기나 코딩 도우미에 관계없이 모든 기여자에게 적용되는 규칙을 정의합니다. AI의 도움을 받은 변경을 포함해, 제출한 코드를 이해하고 테스트할 책임은 기여자에게 있습니다.

도구별 지침 파일(`AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.cursorrules` 등)을 저장소에 추가하지 마세요. `.gitignore`를 우회하여 강제로 추가하더라도 CI가 흔한 에이전트 설정 경로를 거부합니다. 개인용 도우미 설정은 저장소 밖에 보관하세요. 공통 규칙이 빠졌다면, 이유를 설명하는 별도 PR에서 이 문서 또는 관련 공개 문서의 변경을 제안하세요. 유지 관리자가 기여 규칙과 CI 변경을 검토합니다.

## 라이선스

기여하면 제출한 변경 사항에 [Apache License 2.0](LICENSE)이 적용되는 데 동의하는 것입니다.
