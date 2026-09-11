# Windows LUKS2 Read-only Bridge 설계

- 문서 버전: 0.2.0 / 2026-09-10
- 상태: **설계 검토용 초안. 구현·Windows 실행·실제 디스크 검증은 수행하지 않았다.**
- 문서 검토: 0.1.0의 독립 검토 수정사항을 유지하고, 0.2.0의 ext4 추가분에도 제한된 독립 정적 검토 1회를 수행했다. 전달을 막는 설계 모순은 발견되지 않았으며 RO 관찰 경계와 초기 설정 시점 등을 작성자가 소스와 대조해 명확히 했다. 구현·배포 승인이나 런타임 안전성 검증을 뜻하지 않는다.
- 작업명: `winluks2-ro` (가칭이며 기존 제품명이 아니다.)
- 요청 범위: Markdown 설계 문서 작성. 코드 구현, 설치, GitHub 게시, 디스크 접근은 포함하지 않는다.
- 핵심 결정: **LUKS2 → WinSpd 공통 블록 계층 위에 Btrfs는 WinBtrfs, ext4는 Ext4Fsd 후보를 각각 연결한다. 두 파일시스템 드라이버는 수정하지 않는다.**
- 0.2.0 변경: ext4 지원 프로파일·게시 전 검사·저널 복구 거부·별도 Windows 게이트를 추가했다. 0.1.0 원본은 보존한다. 문서 버전 변경은 4096바이트 LUKS 섹터나 RW 지원 추가를 뜻하지 않는다.

## 1. 요약과 성공 조건

목표는 Linux에서 만든 LUKS2 암호화 볼륨의 **Btrfs 또는 ext4** 파일을 Windows에서 읽고 다른 저장장치로 복사하는 것이다. Btrfs는 WinBtrfs, ext4는 별도 드라이버인 **bobranten/Ext4Fsd**를 우선 검증 후보로 삼는다. WinBtrfs가 ext4를 처리한다고 가정하지 않는다. WinSpd와 LUKS2 메타데이터·키 복구·섹터 복호화·읽기 전용 백엔드는 공통으로 재사용한다. 각 세션은 파일시스템 하나만 선택한다.

WinBtrfs 개발자는 LUKS2 복호화 장치를 제공하는 별도 드라이버가 필요하다고 설명했다.[12] WinSpd는 사용자 모드에서 SCSI 디스크를 제공하는 구조이며, `WriteProtected`와 읽기 콜백을 제공한다.[1][2][9] 따라서 새 커널 드라이버 없이 시도할 수 있다. Ext4Fsd README는 Windows 10/11용 signed release와 metadata checksum·64-bit block number 지원을 안내한다.[13] **이는 프로젝트의 주장과 배포 안내를 확인한 것이며, WinSpd/WinBtrfs/Ext4Fsd 바이너리의 서명·호환성·연동은 모두 미검증이다.**

첫 출시 후보의 성공 조건은 다음과 같다.

1. 지원 프로파일의 테스트 이미지에서 비밀번호를 검증하고, Linux `cryptsetup`으로 연 결과와 동일한 평문 바이트를 읽는다.
2. Windows에서 해당 이미지를 읽기 전용 장치로 게시하여, Btrfs/WinBtrfs와 ext4/Ext4Fsd **각각**에서 파일을 읽고 복사한다. 한 경로의 성공을 다른 경로의 지원 증거로 세지 않는다.
3. 정상 동작, 잘못된 비밀번호, 쓰기 시도, 종료·오류 시험 전후에 **암호화 원본 이미지 전체 해시가 동일하다.**
4. 미지원 포맷, 손상된 헤더, 재암호화 상태를 추측하여 열지 않는다.
5. 기능 검증과 별개로 보안 검토·오류 주입·실제 Windows 통합 검증을 완료한다.

**시제품의 동작 확인은 실제 원본 디스크를 안전하게 사용할 수 있다는 보증이 아니다.** 초기 결과물에는 실제 파티션 접근 기능을 넣지 않는다.

## 2. 범위와 사용자 환경에 대한 가정

### 2.1 v0.2의 명시적 지원 프로파일

범용 LUKS2 구현이 아니라 다음 프로파일을 허용하는 구현이다. 아래 항목은 현재 소프트웨어의 지원 실적이 아닌 설계상 목표다.

- Windows 11 x64의 **검증한 특정 OS 빌드**와 공통 WinSpd, 파일시스템별 WinBtrfs 또는 Ext4Fsd 바이너리 조합. 두 경로는 독립적으로 검증한다.
- 로컬의 일반 파일 하나를 백엔드로 사용한다. 파일 내용의 바이트 0에서 LUKS2 헤더가 시작한다.
- 외부 GPT/MBR 디스크 전체 이미지가 아니라, **LUKS2 파티션 내용만 담은 이미지**를 입력으로 받는다.
- 헤더가 데이터와 같은 이미지에 존재하는 LUKS2, `crypt` 데이터 세그먼트 하나.
- 데이터 암호는 `aes-xts-plain64`, 전체 XTS 볼륨 키 길이는 32 또는 64바이트.
- **데이터 암호화 섹터 크기는 512바이트만 허용한다.** 4096바이트 지원은 별도 확장 게이트로 남긴다.
- 데이터 `iv_tweak`는 0만 허용한다. 0이 아닌 경우 지원하지 않는다고 명시적으로 거부한다.
- 세그먼트 크기는 유효한 바이트 수 또는 `dynamic`이다. `dynamic`은 열 때 고정한 이미지 크기로 한 번 계산한다.
- `luks2` 키슬롯의 `raw` 영역, `aes-xts-plain64` 키슬롯 암호, `luks1` AF 방식을 지원한다.
- 키슬롯 KDF는 PBKDF2-HMAC-SHA256/SHA512, Argon2i/Argon2id를 대상으로 한다.
- AF hash는 SHA256/SHA512, 볼륨 키 digest는 PBKDF2-HMAC-SHA256/SHA512를 대상으로 한다.
- 키슬롯 ID를 사용자가 선택한다. 자동으로 모든 슬롯을 순회하지 않는다.
- 암호화·재암호화가 완료되고 Linux에서 정상적으로 언마운트된, LUKS 데이터 영역 바로 위의 **단일 장치 Btrfs 또는 ext4**를 사용한다. LVM·MD·중첩 파티션·파일시스템 이미지 컨테이너는 해석하지 않는다.
- Btrfs는 고정 WinBtrfs 지원 범위, ext4는 §7.3의 보수적 E0 프로파일과 고정 Ext4Fsd 시험 결과에 추가로 제한된다.[8][13]
- 한 프로세스가 이미지 하나, 가상 장치 하나를 관리한다. 초기 동시 요청 처리기는 하나다.

**사용자의 실제 볼륨이 이 프로파일에 맞는지는 확인하지 않았다.** 특히 4096바이트 섹터 또는 다른 KDF/hash 설정이면 v0.2로는 열 수 없다. 호환성을 맞추려고 실제 볼륨을 재포맷하거나 키 설정을 변경하지 않는다. 필요하면 구현 프로파일을 확장한다.

### 2.2 제외 범위

- 실제 파티션, `PhysicalDrive` 등 장치 경로, 네트워크 파일, 파일 안의 임의 offset.
- 쓰기, TRIM/UNMAP, 헤더 복구·수정, 키슬롯 추가·삭제, 비밀번호 변경, 암호화·재암호화.
- detached header, 다중 데이터 세그먼트, dm-integrity/인증 암호화, OPAL 하드웨어 암호화.
- TPM/FIDO2/PKCS#11 등 토큰 실행, 키링, 자동 잠금 해제, 외부 플러그인 실행.
- 시스템 부팅 볼륨, 상시 서비스, 다중 사용자 제공, 성능 최적화, GUI 설치 도구.
- 절전·최대절전 중 열린 볼륨 유지와 안전한 키 회수 보장.
- 손상된 Btrfs/ext4 복구, ext4 journal replay·orphan cleanup·fsck·fast commit replay, 미완료 트랜잭션 복구, multi-device/RAID 구성.
- ext4의 fscrypt는 LUKS2와 다른 파일 단위 암호화 계층이며 지원하지 않는다. LUKS2 키로 fscrypt 파일을 자동 해독하지 않는다.

토큰 메타데이터가 존재한다는 이유만으로 토큰 코드를 실행하지 않는다. 선택한 비밀번호 슬롯과 단일 데이터 세그먼트의 연결을 독립적으로 검증한다.

## 3. 대안과 결정 기록

### ADR-001: 암호화 계층을 파일시스템 밖에 둔다

- 대안 A: WinBtrfs에 LUKS2 처리와 암호화를 직접 추가한다.
  - 파일시스템 코드와 블록 암호화 수명주기가 결합되고, 업스트림 병합·유지보수 부담이 커진다.
- **선택 B: 별도 사용자 모드 프로그램 + WinSpd + 수정하지 않은 파일시스템 드라이버(WinBtrfs 또는 Ext4Fsd).**
  - 커널 코드 신규 작성량을 줄이고 Linux differential test가 가능한 독립 코어를 만든다.
  - 대신 사용자/커널 경계의 복사·스케줄링 비용, 외부 드라이버의 유지보수·서명 의존성이 생긴다.
- 대안 C: Linux VM에서 `cryptsetup`으로 열고 SMB/SFTP로 제공한다.
  - 직접 구현이 목적이 아니라 파일 접근만 필요하다면 이쪽이 더 현실적인 운영 대안이다.

선택 B가 G0의 드라이버·장치 인식 검증을 통과하지 못하면, 읽기 전용 원칙을 완화하지 않는다. 원인을 확인한 뒤 대안이나 별도 설계 변경을 검토한다.

### ADR-002: 파일 이미지와 읽기 전용부터 시작한다

원본 디스크와 쓰기 경로를 초기 구현에서 제거한다. `--write`나 `--force`로 보호를 끄는 숨겨진 옵션도 두지 않는다. raw device/RW 확장은 별도 설계와 승인 대상이다.

### ADR-003: 엄격한 지원 프로파일을 사용한다

"LUKS2"라는 이름만 보고 모든 변형을 지원한다고 가정하지 않는다. 알려진 구조를 해석한 뒤 선택된 슬롯·digest·세그먼트를 검증하고, 지원 여부를 판단한다. 미지원 기능은 잘못된 비밀번호와 구분하여 반환한다.

### ADR-004: 암호 알고리즘을 새로 작성하지 않는다

권장 구현 언어는 메모리 안전성이 높은 Rust이며, WinSpd와 연결하는 최소 C ABI shim을 둔다. AES-XTS, PBKDF2, hash는 Windows 빌드가 가능한 검증된 라이브러리(후보: OpenSSL 3 계열), Argon2는 표준 구현을 재사용한다. **최종 라이브러리·버전·빌드 옵션·라이선스·키 제거 동작은 G1 전에 고정한다.** 라이브러리 사용 자체가 안전성 감사를 대신하지 않는다.

### ADR-005: ext4는 별도 소비자와 게시 전 검사를 둔다

LUKS2 코어는 파일 이름이나 inode를 해석하지 않는 공통 읽기 전용 블록 계층으로 유지한다. `fs-probe`는 파일시스템 정책을 확인하는 얇은 게시 전 검사이며, 새 ext4 파일시스템 드라이버나 복구 도구가 아니다.

- **선택:** ext4는 `bobranten/Ext4Fsd`를 우선 검증한다. WinBtrfs에는 ext4 기능을 추가하지 않는다. 과거 Ext2Fsd/Ext3Fsd의 지원 설명이나 동일한 `Ext2Fsd` 바이너리 이름을 검증한 Ext4Fsd 릴리스의 증거로 사용하지 않는다.[13]
- **비선택:** ext4 전체를 직접 구현하거나 사용자 모드 ext4 라이브러리와 WinFsp를 결합하는 방식은 inode·경로·파일 API 계층과 라이선스 검토를 새로 요구하므로 이번 범위에 넣지 않는다.
- **운영 대안:** G0-E를 통과하지 못하면 ext4는 차단된 후보로 남긴다. Linux VM의 `cryptsetup`+ext4+SMB/SFTP는 별도 대안이지 자동 fallback이 아니다.
- 파일시스템별 검사/드라이버/시험 결과를 분리한다. Btrfs 성공, ext4 드라이버 설치 성공, 장치가 보임 중 어느 하나도 ext4 지원 완료를 뜻하지 않는다.

## 4. 아키텍처와 신뢰 경계

```text
[Windows 앱 / 탐색기]
          │ 파일 읽기
          ▼
[선택한 기존 커널 파일시스템 드라이버]
  Btrfs → WinBtrfs / ext4 → Ext4Fsd 후보
          │ 블록 요청
          ▼
[WinSpd: 기존 가상 SCSI miniport + 사용자 모드 DLL]
          │ Read / Write / Flush / Unmap callbacks
          ▼
[winluks2-ro.exe: 단일 사용자 모드 프로세스]
  ├─ interactive-cli: 로컬 콘솔 입력·상태 출력
  ├─ lifecycle: 시작·게시·종료·오류 상태 전이
  ├─ winspd-adapter: C ABI, geometry, 상태 코드 변환
  ├─ fs-probe: 키 검증 후 파일시스템 타입·허용 프로파일 검사
  ├─ luks2-core: 헤더·JSON·키슬롯·digest 검증
  ├─ crypto-provider: KDF / AF merge / AES-XTS
  └─ image-backend: 읽기 전용 파일 핸들·범위 제한
          │ 오직 원본 바이트 읽기
          ▼
[로컬 암호화 이미지 파일: 신뢰하지 않는 입력]
```

- 이미지의 모든 바이트, JSON 필드, KDF 파라미터, 파일 경로, 요청 LBA는 신뢰하지 않는 입력이다.
- 선택한 WinBtrfs/Ext4Fsd와 공통 WinSpd는 커널 신뢰 기반에 포함된다. 이 프로그램이 외부 드라이버 결함까지 차단하지는 못한다.
- 잠금 해제된 평문은 Windows 스토리지 스택과 접근 권한이 있는 앱에 전달된다. v0.2는 공유 PC의 사용자 간 기밀성 격리 기능이 아니다.
- 로컬 관리자·커널 공격자, DMA·콜드부트 공격, 잠금 해제 후 악성코드의 파일 읽기는 보호 범위 밖이다.
- AES-XTS는 데이터 기밀성을 제공하지만 인증 암호가 아니다. 헤더 체크섬과 Btrfs/ext4 체크섬은 공격자에 대한 암호학적 무결성 보증을 제공하지 않는다.
- 읽기 전용 장치도 악성 파일시스템 메타데이터를 커널 드라이버에 전달한다. v0.2는 악성 이미지 분석용 sandbox가 아니며, 신뢰할 수 없는 이미지의 마운트 시험은 폐기 가능한 격리 VM에서만 수행한다. 파일 공유 제한은 임의의 기존 writable mapping이나 관리자 수준 쓰기까지 차단하는 storage snapshot 보장이 아니다.

### 4.1 모듈 계약

아래 이름은 제안하는 내부 API이며 이미 존재하는 함수가 아니다.

- `ImageBackend::open_readonly(path) -> ImageSnapshot`
  - 일반 로컬 파일인지 확인하고 파일 ID·크기·읽기 핸들을 고정한다. 쓰기/삭제 공유를 허용하지 않는다.
  - 경로 검사 후 재개방하지 않는다. 실제 연 핸들의 속성을 검증하여 경로 교체/TOCTOU를 줄인다.
  - 장치·파이프·원격 파일·reparse 기반 우회 입력은 정책에 따라 거부한다. 이 제한은 관리자에 대한 방어가 아니다.
- `parse_metadata(snapshot, limits) -> ValidatedMetadata`
  - 출력은 숫자 범위와 관계가 검증된 불변 구조다. 아직 키를 포함하지 않는다.
- `select_unlock_plan(metadata, slot_id) -> UnlockPlan`
  - 슬롯→digest→단일 데이터 세그먼트의 연결과 지원 프로파일을 검증한다.
- `unlock(plan, password_bytes) -> UnlockedVolume`
  - KDF·키슬롯 복호화·AF merge·digest 검증에 성공한 경우에만 볼륨 키를 반환한다.
- `UnlockedVolume::read_at(offset, len) -> plaintext`
  - 평문 볼륨 기준의 범위만 받으며 원본 offset과 IV 계산을 내부에서 수행한다.
- `probe_filesystem(volume, expected_fs, policy) -> ValidatedFsProfile`
  - 키/digest 검증 후 읽기 전용 `read_at`으로 선택한 파일시스템 타입을 대조한다. ext4는 §7.3 검사까지 완료한다. Btrfs는 기존 단일 장치·feature 정책을 유지한다.
  - 검사 결과는 열린 volume identity, 길이, 세션 generation, filesystem kind와 정책 버전에 결합한다. 다른 볼륨의 결과를 재사용하지 않는다. 파서에 write·repair·자동 driver fallback을 제공하지 않는다.
- `publish_readonly(volume, validated_fs_profile) -> Session`
  - 키 검증과 파일시스템 프로파일 검증 모두 성공한 경우에만 WinSpd 장치를 게시한다. 프로파일의 볼륨·generation 결합을 확인한다. 실패한 unlock/probe 중에는 장치가 존재해서는 안 된다.

## 5. 메타데이터 검증과 잠금 해제

### 5.1 헤더와 JSON

LUKS2는 기본/보조 메타데이터와 체크섬, JSON 구조를 가진다.[5][6][7] v0.2는 Linux의 복구 동작 전체를 복제하지 않고 **보수적으로 정상 상태만 허용**한다.

1. 고정 길이 필드부터 읽는다. `magic`, version, header size, offset, endian 변환을 명시적으로 검증한다.
2. 크기를 검증하기 전에 헤더 필드를 신뢰하여 메모리를 할당하지 않는다.
3. 두 헤더의 위치·체크섬·UUID·seqid와 JSON을 독립적으로 검증한다. v0.2는 양쪽이 유효하고 seqid가 같으며 의미상 같은 메타데이터인 경우만 허용한다.
4. 둘 중 하나가 손상되거나 seqid가 다르면 `METADATA_RECOVERY_REQUIRED`로 거부한다. 원본을 자동 복구하거나 오래된 복사본을 임의 선택하지 않는다. 이 정책은 유효한 일부 LUKS2 복구 상황도 의도적으로 제외한다.
5. JSON은 중복 객체 키, 잘못된 타입, 과도한 중첩, 잘못된 UTF-8, 숫자 변환 overflow를 거부한다. 바이트 offset처럼 문자열로 저장된 수치를 부동소수점으로 변환하지 않는다.
6. 헤더/JSON과 키슬롯 영역, 데이터 영역의 범위와 overlap을 검증한다. base64, AF stripes, key size와 영역 용량의 관계도 확인한다.
7. 미지원 필수 requirement, 재암호화 키슬롯·세그먼트·플래그, integrity/OPAL 형태를 검출하면 장치 게시 전에 거부한다. 단순히 세그먼트 개수만 검사해서 재암호화 완료를 판단하지 않는다.[6]
8. `config.requirements` object 안의 `mandatory` array를 포함해 동작에 영향을 주는 객체를 허용 목록 방식으로 검사한다. requirements 구조는 고정 cryptsetup JSON 검증 소스를 기준으로 하며, 다른 JSON 형태로 오해하여 요구사항을 누락하지 않는다.[6] 비실행 label/subsystem 정보는 길이 제한 후 표시할 수 있다. 모르는 동작 필드를 조용히 무시하지 않는다.

초기 자원 제한은 **제품 정책 후보**이며 LUKS2 규격상의 최대치가 아니다.

- 메타데이터 복사본당 최대 4 MiB, JSON 중첩 최대 32단계.
- 동시에 파생하는 키는 하나이며 KDF 작업자도 하나다.
- Argon2 최대 메모리 1 GiB, time cost 10, lanes 8; PBKDF2 최대 10,000,000 iterations.
- 선택한 키슬롯의 암호화 영역과 AF 임시 버퍼는 각각 최대 64 MiB로 제한한다. `af.type=luks1`의 `stripes`는 **정확히 4000**만 허용한다. 이는 자원 상한과 별개인 LUKS2 포맷 계약이며, 다른 값은 `UNSUPPORTED_PROFILE`로 거부한다.[4][7] 전체 슬롯 수는 32개까지 해석한다. 크기·stripes·key size 곱을 검증한 뒤에만 할당한다.
- KDF에는 120초의 **soft deadline**을 적용한다. 취소가 가능한 라이브러리는 협력적으로 취소한다. 취소가 불가능하면 제한된 파라미터의 계산이 끝날 때까지 자원을 유지하고, deadline 이후 결과는 폐기하여 장치를 게시하지 않는다. 단일 프로세스 v0.2에서 정확히 120초 이내 종료를 보장하지 않으며 임의 thread kill은 금지한다.
- 비밀번호 입력 최대 4096 UTF-8 바이트. 파라미터·버퍼 한도 또는 soft deadline 초과는 `RESOURCE_LIMIT`로 반환한다.

키슬롯 수·AF stripes·키 영역의 총 작업량도 checked arithmetic와 검증된 영역 크기로 제한한다. 위 한도를 조정할 필요가 있으면 비밀이 없는 로컬 정책 변경과 재시험을 거친다. **기록된 KDF 파라미터를 낮춰 계산하거나 잠금 해제를 우회하지 않는다.**

### 5.2 비밀번호와 키슬롯

- 비밀번호는 로컬 콘솔에서 echo 없이 받는다. argv, 환경변수, 설정 파일, 로그로 받지 않는다.
- Windows 문자열은 엄격한 UTF-8로 변환하며 Unicode normalization을 임의 적용하지 않는다. 잘못된 surrogate는 거부한다.
- v0.2는 Linux에서 같은 UTF-8 바이트로 사용한 비밀번호를 대상으로 한다. 임의 binary keyfile은 제외한다.
- `Kvol = keyslot.key_size`와 `Kslot = keyslot.area.key_size`는 서로 다른 길이다. 둘이 같다고 가정하지 않으며 v0.2는 각각 독립적으로 32 또는 64바이트만 허용한다. 선택 슬롯의 KDF 출력은 **Kslot바이트**, AF 병합 결과와 데이터 볼륨 키는 **Kvol바이트**다.[4][7]
- `AFBytes = checked(round_up(Kvol × 4000, 512))`가 `area.size` 이하인지 확인한다. 검증한 슬롯 `area.offset`에서 AFBytes만 읽고 복호화한 뒤 4000 stripes를 AF merge하여 Kvol바이트 볼륨 키 후보를 복구한다. 영역 예약 크기와 실제 AF 데이터 길이를 혼동하지 않는다.[4]
- 키슬롯 암호화는 선택한 프로파일에서 **512바이트 단위, 슬롯 영역 시작을 IV 기준 0으로 사용**한다. 물리 이미지의 절대 섹터 번호나 데이터 세그먼트의 IV를 재사용하지 않는다. cryptsetup의 키슬롯 읽기 경로가 이 분리를 보여 준다.[4]
- 복구한 키는 해당 슬롯과 데이터 세그먼트에 연결된 digest로 검증한다. Btrfs/ext4 magic이 보인다는 이유만으로 키 검증을 성공 처리하지 않는다.
- digest 실패는 `UNLOCK_FAILED`다. 비밀번호 오류와 암호화 키 영역 손상을 확정적으로 구분할 수 있다고 주장하지 않는다.
- 성공/실패 후 비밀번호, KDF 출력, AF 중간 버퍼를 제거한다. 볼륨 키는 열린 세션에만 유지한다.

## 6. 읽기 경로와 주소 계산

### 6.1 주소 공간

세 종류의 offset을 이름과 타입으로 구분한다.

- `image_offset_bytes`: LUKS2 헤더를 포함한 원본 이미지의 위치.
- `volume_offset_bytes`: 복호화된 데이터 세그먼트 시작을 0으로 하는 위치.
- `iv_sector_index`: XTS에 전달하는 IV 계산용 번호.

v0.2에서는 가상 장치의 LBA 0이 복호화된 볼륨의 바이트 0이다. LUKS2 헤더·키슬롯 영역은 가상 디스크에 노출하지 않는다.

```text
S = 512                                      # v0.2 고정 암호화 섹터 크기
B = 512                                      # WinSpd BlockLength
D = validated segment.offset                 # 원본 내 데이터 시작 바이트
L = validated data length                    # 평문 볼륨 전체 바이트
require D % S == 0 and L > 0 and L % S == 0
require D <= pinned_image_size and L <= pinned_image_size - D
V = checked(BlockAddress * B)
N = checked(BlockCount * B)
require V <= L and N <= L - V
source_start = checked(D + V)
require source_start + N <= pinned_image_size

각 S 바이트 데이터 단위의 시작 위치 p에 대해:
  source_offset = D + p
  iv_sector_index = p / 512                   # iv_tweak=0인 지원 프로파일
  IV = LE64(iv_sector_index) || zero[8]
  plaintext_unit = AES-XTS-decrypt(volume_key, IV, ciphertext_unit)
```

`plain64`의 little-endian IV 생성과 암호화 섹터별 처리 방식은 cryptsetup 구현을 기준으로 한다.[10] **하나의 큰 읽기 요청을 XTS data unit 하나로 처리하면 안 된다.** 섹터마다 IV를 다시 계산한다. 패딩을 사용하지 않는다.

- D/L/V/N 및 모든 덧셈·곱셈은 overflow를 검사한다. 요청 끝이 볼륨을 넘으면 일부 성공이 아니라 오류다.
- 원본 read의 short read/EOF를 성공이나 0 채움으로 숨기지 않는다.
- 요청 전체가 성공한 경우만 평문을 완료 버퍼에 반영한다. 실패 시 전송 버퍼와 임시 평문을 정리하고 실패 상태로 완료한다.
- 초기 `MaxTransferLength` 정책은 1 MiB다. 큰 요청 분할과 거부 동작은 WinSpd 통합 시험으로 확인한다.
- zero-block 요청, 마지막 LBA, 범위 밖 요청의 의미는 고정 SDK와 테스트에서 확정한다. zero-block 일반 읽기는 범위를 검사한 뒤 no-op으로 처리하되 Flush 등 다른 명령과 의미를 섞지 않는다.

### 6.2 4096바이트 확장 조건

4096바이트 LUKS 암호화 섹터는 목표 사용 환경에서 필요할 가능성이 있지만 v0.2의 지원 목표에 포함하지 않는다. ext4의 4096바이트 파일시스템 블록과는 별개다. 확장 전에 다음을 별도 ADR과 Linux 비교 벡터로 확정한다.

- LUKS2의 sector size와 dm-crypt의 IV numbering/large-IV 의미를 소스와 실제 매핑에서 확인한다.
- 데이터 섹터 크기와 **키슬롯의 512바이트 암호화 단위**를 독립적으로 처리한다.
- WinSpd logical block을 512 또는 4096 중 무엇으로 광고할지 결정한다.
- 512바이트 읽기가 4096바이트 암호화 단위 일부를 요구하면 전체 단위를 복호화한 뒤 필요한 부분만 반환한다.
- 여러 단위에 걸친 비정렬 범위, 마지막 섹터, nonzero IV 설정을 검증한다.

v0.2 식에 512를 4096으로 일괄 치환하여 지원을 추가하지 않는다.

## 7. WinSpd 연동과 읽기 전용 불변식

확인한 `SPD_IOCTL_STORAGE_UNIT_PARAMS`에는 `BlockCount`, `BlockLength`, `WriteProtected`, `CacheSupported`, `UnmapSupported`, `MaxTransferLength`가 있다.[2]

게시 시 다음을 설정한다.

- `BlockLength=512`, `BlockCount=L/512`, geometry를 세션 내 고정한다.
- `WriteProtected=1`, `CacheSupported=0`, `UnmapSupported=0`.
- 고유 가상 장치 GUID를 사용한다. 재연결 시 이전 세션의 키/요청이 새 세션에 연결되지 않게 한다.

읽기 전용 보호는 서로 다른 계층에서 강제한다.

1. 이미지 핸들은 `GENERIC_READ`로만 연다. 라이브러리에 원본 경로를 전달하여 쓰기로 재개방하게 하지 않는다.
2. 백엔드 인터페이스 자체에 write/resize/discard 메서드를 두지 않는다.
3. WinSpd에 write-protected 디스크임을 광고한다.
4. `Write`와 `Unmap` 콜백에도 방어적 거부를 구현한다. 예상 밖 요청이 도달해도 원본을 수정하지 않는다.
5. WinBtrfs에는 per-volume `Readonly=1` 설정을 추가 방어로 사용한다. 문서상 설정 적용에 reboot가 필요하므로 첫 자동 마운트보다 먼저 적용되도록 테스트 VM에서 절차를 검증한다.[8]

6. ext4는 §7.3의 Ext4Fsd RO 정책을 첫 게시 전에 적용·검증한다. `WritingSupport`와 force-write를 끄고, 실제 선택 볼륨의 RO 상태도 확인한다. WinBtrfs의 registry 경로/설정 적용 시점을 Ext4Fsd에 복사하지 않는다.

**파일시스템 드라이버의 RO 설정은 원본 보호의 유일한 수단이 아니다.** 파일시스템이 쓰기를 요구하여 마운트에 실패하면 보호를 해제하지 않고 실패를 보고한다.

### 7.1 콜백 계약과 오류

WinSpd 콜백의 `BOOLEAN`은 성공/실패가 아니라 **완료 여부**다. `FALSE`는 pending이며 이후 `SpdStorageUnitSendResponse`가 필요하다.[3] 초기 구현은 동기 완료만 사용한다.

- 정상 Read: 상태를 성공으로 설정하고 `TRUE`를 반환한다.
- Write: DATA PROTECT / WRITE PROTECTED로 완료하고 `TRUE`를 반환한다.
- Unmap: 미지원/보호 오류로 완료한다. 성공한 것처럼 응답하거나 암호문을 0으로 채우지 않는다.
- 범위 오류: ILLEGAL REQUEST 계열로 완료한다.
- 백엔드 I/O 오류: 적절한 MEDIUM ERROR 또는 NOT READY로 완료한다. 구체적인 SCSI sense 매핑은 SDK 상수와 테스트로 고정한다.
- Flush: 쓰기가 없고 dirty cache가 없으므로 활성 RO 장치에서 범위를 검증한 no-op 성공이다. 이는 RW durability 구현이 있다는 뜻이 아니다.
- Read의 FUA/flush 성격 플래그도 이 RO/no-dirty-cache 정책과 일치하게 처리한다.[3]

상태는 호출마다 초기화한다. Rust panic/예외가 C ABI 경계를 넘지 않게 하며, 실패를 pending으로 잘못 반환하여 I/O를 영구 대기시키지 않는다.

### 7.2 디스크 레이아웃과 마운트 발견: 선행 게이트

선택한 최초 레이아웃은 **파티션 테이블 없는 복호화 볼륨을 전체 가상 SCSI 디스크로 노출**하는 형태다. WinBtrfs와 Ext4Fsd가 이를 각각 정상 발견하고 mount manager가 볼륨을 노출하는지는 G0-B/G0-E에서 독립적으로 확인한다.

실패 시 Windows 디스크 관리의 "초기화/포맷"을 실행하지 않는다. 필요하면 이후 설계에서 메모리 내 합성 GPT로 선택한 파일시스템을 감싸는 대안을 검토할 수 있으나, v0.2의 자동 fallback은 아니다. GPT를 추가하면 virtual LBA와 파일시스템 volume offset의 변환, 앞/뒤 GPT와 CRC·geometry 테스트가 새로 필요하다.

클론과 원본의 파일시스템 UUID는 같을 수 있다. G0 및 통합 시험에서는 같은 UUID의 다른 장치를 Windows에 동시에 노출하지 않는다.

### 7.3 ext4 E0 프로파일과 게시 전 거부 규칙

#### 드라이버 근거와 불확실성

Ext4Fsd README는 extents·directory indexing·hardlink/symlink·internal journal replay와 metadata checksum·64-bit block number 지원을 안내한다.[13] 소스에는 media write protection을 확인하고 RO 플래그를 설정하는 경로가 있으며,[14][20] `Ext2CheckJournal`은 `IsVcbReadOnly`이면 journal load 전에 중단한다.[15] **이는 소스에서 확인한 조건이지, 배포 바이너리가 이 가상 장치에서 항상 RO가 된다는 증명이 아니다.**

특히 README에는 CASEFOLD가 미지원으로 나오는 반면, 조회한 HEAD의 incompat 지원 mask에는 CASEFOLD가 포함되어 있다.[13][16] 소스 HEAD와 signed 0.71 release의 동일성도 확인하지 않았다. 이러한 차이는 자동으로 유리하게 해석하지 않으며 **E0에서 CASEFOLD를 거부**한다. 0.71 다운로드 안내와 서명 여부는 G0-E에서 실제 바이너리의 hash·버전·서명자·서명 체인을 확인해야 한다.

#### E0 허용 조건: 의도적으로 좁은 제품 정책

다음 조건의 교집합만 ext4 후보로 인정한다. 모든 ext4나 배포판 기본 `mkfs.ext4` 결과를 지원한다는 뜻이 아니다. feature 이름과 분류는 고정 헤더 및 Linux on-disk 문서에 근거한다.[16][18]

- LUKS 데이터 영역 바로 위에 단일 ext4가 있고, primary superblock은 **평문 볼륨 offset 1024의 1024바이트 영역**에서 읽는다. ext magic만으로 ext2/ext3/ext4를 구분하지 않는다.
- `s_magic=0xEF53`, dynamic revision 1, inode size 256바이트, **ext4 block size 4096바이트**를 초기 프로파일로 선택한다. 다른 크기는 명시적으로 미지원 처리한다.
- **ext4 block 4096 ≠ LUKS 암호화 sector 4096.** E0의 LUKS data unit과 WinSpd logical block은 기존대로 각각 512바이트다. ext4 block 하나를 읽을 때 공통 계층이 8개의 암호화 data unit을 처리하며 XTS IV를 섞지 않는다.
- required compat: `HAS_JOURNAL`; optional compat: `EXT_ATTR`, `RESIZE_INODE`, `DIR_INDEX`.
- required incompat: `FILETYPE`, `EXTENTS`; optional incompat: `64BIT`, `FLEX_BG`, `CSUM_SEED`.
- required ro_compat: `METADATA_CSUM`; optional ro_compat: `SPARSE_SUPER`, `LARGE_FILE`, `HUGE_FILE`, `DIR_NLINK`, `EXTRA_ISIZE`.
- 세 feature class 모두 위 목록 밖의 bit가 있으면 거부한다. compat/ro_compat의 일반적인 전방 호환 규칙보다 좁은 **실험판 정책**이다. `CSUM_SEED`는 metadata checksum과 함께 해석하고, `GDT_CSUM`과 `METADATA_CSUM`을 동시에 허용하지 않는다.
- journal은 내부 journal만 허용한다. `s_journal_inum != 0`, `s_journal_dev == 0`, external journal UUID가 없는 구성을 요구한다. 별도 journal 장치를 찾아 열지 않는다.
- `64BIT` 유무에 따라 blocks count 상위 필드·group descriptor 크기를 해석한다. E0은 64BIT이면 descriptor 64바이트, 아니면 규격의 기본 descriptor 32바이트를 사용한다. 64BIT가 꺼진 경우 디스크의 `s_desc_size` 필드가 반드시 32여야 한다고 요구하지 않는다. blocks count 상위 필드를 조용히 버리지 않는다.
- `s_checksum_type=1`(CRC32c)을 요구한다.[18]
- checked arithmetic로 `filesystem_blocks × 4096 <= plaintext_volume_length` 및 관련 group/inode geometry를 검증한다. primary superblock checksum을 검증하고, 알 수 없는 checksum type·잘린 구조·불가능한 geometry는 거부한다. checksum 계산은 고정 Linux/e2fsprogs 규칙 및 독립 벡터로 검증하며 새 알고리즘을 추정하지 않는다.

즉 `INLINE_DATA`, `ENCRYPT`(fscrypt), `CASEFOLD`, `EA_INODE`, `MMP`, `BIGALLOC`, quota/project quota, `VERITY`, `FAST_COMMIT`, `ORPHAN_FILE`/`ORPHAN_PRESENT`, `META_BG`, `LARGEDIR`, `SPARSE_SUPER2` 등은 E0에서 제외한다. 일부 기능을 Ext4Fsd가 RO로 허용하거나 소스에 포함하더라도 E0 통과로 간주하지 않는다. 특히 최근 e2fsprogs 기본값에 orphan_file 등이 포함될 수 있어 정상 Linux 볼륨도 거부될 수 있다. 실제 사용자 볼륨의 기능을 끄거나 재포맷하여 이 프로파일에 맞추지 않는다.

#### clean 상태와 journal replay

Linux 공식 문서는 **ext4를 `ro`로 마운트해도 journal replay로 쓸 수 있음**을 명시한다. `ro,noload`는 replay를 막지만 dirty filesystem의 일관성을 회복해 주지 않는다.[17][19] 따라서 읽기 전용만으로 복구가 불필요하다고 추정하지 않는다.

키 검증 후, 장치 게시 전에 다음 순서로 검사한다.

1. 구조·범위·타입·checksum 검사 실패는 `FS_INVALID` 또는 `FS_TYPE_MISMATCH`로 거부한다.
2. `s_state`의 VALID_FS bit가 없거나 ERROR_FS bit가 있으면 `FS_RECOVERY_REQUIRED`로 거부한다. 알려지지 않은 상태 bit도 E0에서 거부한다.
3. `INCOMPAT_RECOVER`(needs_recovery), `s_last_orphan != 0`, `RO_COMPAT_ORPHAN_PRESENT` 중 하나라도 있으면 `FS_RECOVERY_REQUIRED`로 거부한다. 일반 feature mask 검사보다 먼저 분류한다.
4. 그 밖의 feature/geometry 프로파일 불일치는 `FS_UNSUPPORTED_FEATURE`로 거부한다. ORPHAN_FILE은 clean 여부와 별개로 E0에서 미지원이다.
5. 모두 통과한 경우에만 `FS_PROFILE_VALID`를 만들고 동일한 열린 볼륨을 게시한다. 검사 중 새 경로를 열거나 다른 이미지로 바꾸지 않는다.

이 검사는 전체 ext4 consistency check나 공격자에 대한 authenticity 검증이 아니다. clean 표식이 위조되거나 손상이 다른 구조에 남을 수 있으므로, 파일 읽기 오류를 성공으로 숨기지 않고 악성 이미지 시험은 격리 VM에서만 수행한다. journal replay·fsck·orphan cleanup은 Windows bridge에서 구현하거나 시도하지 않는다. 복구가 필요하면 세션을 닫고 원본을 보존한 채 별도 복제본을 Linux에서 처리하는 절차를 새로 승인받는다.

#### RO와 동작 의미

- Ext4Fsd의 실제 `Readonly`, `WritingSupport`, `Ext3ForceWriting` 및 volume별 설정 적용 경로를 선택 바이너리 기준으로 확인한다.[14][20] 조회한 소스에서는 volume별 registry 적용(`Ext2PerformRegistryVolumeParams`)이 journal recovery 호출보다 뒤에 있다. 따라서 **volume별 Readonly 설정만으로 mount 초기 RO를 보장하지 않는다.** journal 경로 진입 전에 device write-protection과 안전한 초기/global 설정이 적용됐는지 확인하고, 쓰기/force-write를 끈다. CLI나 GUI 표시만 믿지 않는다.
- WinSpd write protection이 Ext4Fsd의 `VCB_WRITE_PROTECTED`/`IsVcbReadOnly`로 전달되는지 G0-E에서 확인한다. RO가 확인되지 않거나 driver가 clean fixture에도 쓰기를 요구하면 거부한다. 하위 백엔드 쓰기 권한을 주어 문제를 우회하지 않는다.
- 자동 mount, journal wipe/replay, mount time·shutdown time·atime 갱신 등 숨은 쓰기 경로를 회귀 시험한다. ext4 자체의 정규 읽기에서 mutation 요청이 발생하면 원본은 하위 계층이 보호하되 **그 드라이버 조합의 게이트는 실패**다. 의도적인 사용자 쓰기 거부 시험과 구분한다.
- mutation 0회 검증은 **RO 차단 이전의 파일시스템 드라이버→스토리지 요청 경계**를 관찰할 수 있어야 한다. WinSpd가 WriteProtected로 먼저 거부하면 사용자 모드 callback 수는 0일 수 있으므로, callback counter 0만을 hidden-write 부재의 증거로 사용하지 않는다. 선택 빌드의 kernel/IRP tracing 또는 동등한 관찰 수단을 G0-E에서 고정하며 관찰할 수 없으면 해당 검증은 미통과로 남긴다.
- 파일 내용 읽기·복사만 목표로 한다. Linux UID/GID·POSIX ACL/xattr의 Windows 보안 등가성, 장치 파일 실행, ext4 verity 검증, fscrypt 해독을 약속하지 않는다.
- Linux에서 가능한 이름이 Windows에서 충돌하거나 표현 불가능하면 명시적 오류/미지원으로 처리한다. 조용한 이름 변경·덮어쓰기·서로 다른 파일을 같은 파일로 표시하는 동작은 허용하지 않는다. hardlink/symlink는 독립 fixture로 검증하고, 링크를 따라 대상 저장장치 밖으로 복사하는 것은 기본 동작으로 두지 않는다. 이는 외부 드라이버·복사 도구의 시험/허용 기준이며, 공통 블록 브리지가 파일명이나 링크를 재작성한다는 뜻이 아니다. 후보 조합이 조용한 충돌·덮어쓰기를 막지 못하면 해당 조합은 지원 게이트에서 탈락한다.

## 8. 수명주기, 종료, 키 보호

```text
CLOSED → IMAGE_OPEN → METADATA_VALID → KEY_VERIFIED
       → FS_PROFILE_VALID → PUBLISHED_RO
       → STOP_REQUESTED → DRAINING → CLOSED

어느 단계에서든 오류 → FAILED → 자원 정리 → CLOSED
```

- 장치는 `KEY_VERIFIED`와 `FS_PROFILE_VALID` 이후에만 생성한다. ext4 dirty/unsupported 입력은 게시 전 실패하고 키를 정리한다. 초기화 중 실패하면 노출된 볼륨이 남지 않게 한다.
- v0.2는 한 dispatcher thread와 bounded synchronous read로 시작한다. 공유 mutable cipher context의 동시 사용을 피한다.
- 정상 종료는 새 요청 유입 차단, 장치 종료 요청, in-flight 작업 완료/취소 확인, dispatcher 종료 대기, 장치 객체 제거, 키·임시 버퍼 제거, 이미지 핸들 닫기 순으로 수행한다.
- SDK의 `SpdStorageUnitShutdown`, `SpdStorageUnitWaitDispatcher`, `SpdStorageUnitDelete`를 이용하되 함수 존재만으로 완료 순서나 callback lifetime을 추정하지 않는다. pinned 구현과 kill/unmount 시험으로 검증한다.[1]
- drain 완료 전 키나 FFI context를 해제하지 않는다. 세션별 generation ID로 과거 요청의 새 세션 접근을 차단한다.
- I/O timeout과 취소는 `CancelIoEx` 등 실제 핸들 동작을 확인하고 설계한다. 무한 대기 또는 callback 실행 중 객체 해제를 허용하지 않는다.
- 프로세스 강제 종료·드라이버 장애에서는 명시적 메모리 zeroization을 보장할 수 없다. 서비스 재시작 시 자동 잠금 해제하지 않는다.
- 정상 종료 시 key/KDF buffer는 최적화로 제거되지 않는 zeroization을 수행한다. 작은 키 영역은 메모리 잠금을 시도하고, 실패는 명시적으로 보고한다. 암호 라이브러리 내부 key schedule 복사본도 검토한다.
- 암호/KDF와 data buffer를 덤프·디버그 로그에 넣지 않는다. Argon2 대용량 작업 영역, Windows 파일 캐시, pagefile, crash dump, hibernation에서 평문 잔존을 완전히 없앤다고 주장하지 않는다.
- 초기 검증 환경은 절전·최대절전을 사용하지 않는다. 열린 볼륨의 suspend/resume 지원은 인증 대상에서 제외한다.

## 9. CLI, 권한, 관측성

아래 CLI는 **제안 인터페이스**이며 현재 실행 가능한 명령이 아니다.

```text
winluks2-ro inspect --image <partition-image-file>
winluks2-ro open --image <partition-image-file> --keyslot <id> --filesystem btrfs --read-only
winluks2-ro open --image <partition-image-file> --keyslot <id> --filesystem ext4 --read-only
# 비밀번호는 로컬 콘솔에서 비표시 입력한다.
# 초기 버전은 foreground 프로세스로 동작하며 종료 요청으로 잠근다.
```

- `inspect`는 키 입력 없이 **LUKS2 계층의 포맷·지원 여부만** 확인한다. 암호화된 내부 파일시스템은 `unknown_locked`로 표시하며 타입·clean 상태·ext4 기능을 추측하지 않는다. salt, digest, keyslot 원문, 전체 JSON을 기본 출력하지 않는다.
- `open`의 `--filesystem`은 필수이며 허용값은 `btrfs`/`ext4`다. 키 검증 뒤 실제 타입과 대조하고 불일치는 거부한다. 실패했다고 다른 드라이버로 재시도하거나 임의 자동 탐지를 수행하지 않는다. 원본 식별 정보, 읽기 전용 정책, 제외 기능을 표시한다. mounted drive letter를 성공 전에 예측하지 않는다.
- WinSpd 설치와 장치 생성의 필요 권한은 G0에서 확인한다. 초기 실행은 단일 사용자의 전용 테스트 VM 관리자 세션으로 한정한다.
- 장기 실행 LocalSystem 서비스와 임의 경로를 받는 IPC는 v0.2에 없다. 추후 서비스화 시 named-pipe ACL, client identity, handle passing을 별도 검토한다.
- 로그 허용 항목: 비식별 세션 ID, 상태 전이, 요청 수·크기·지연, 오류 분류, 버전.
- 로그 금지 항목: 비밀번호, 볼륨 키/derived key, AF buffer, 평문 sector/file 내용. 경로·UUID·label도 공유 로그에는 기본 마스킹한다.
- 성능 수치는 측정값만 기록한다. 초기 성능 목표는 특정 MB/s 보장이 아니라 bounded memory, 무한 대기 없음, 정확한 읽기다.

주요 오류: `FS_TYPE_MISMATCH`, `FS_INVALID`, `FS_RECOVERY_REQUIRED`, `FS_UNSUPPORTED_FEATURE`, `FS_DRIVER_UNAVAILABLE`, `UNSUPPORTED_PROFILE`, `METADATA_INVALID`, `METADATA_RECOVERY_REQUIRED`, `REENCRYPTION_UNSUPPORTED`, `RESOURCE_LIMIT`, `UNLOCK_FAILED`, `BACKEND_IO`, `DEVICE_PUBLISH_FAILED`, `READ_ONLY`, `STOPPING`.

## 10. 검증 전략과 출시 게이트

### 10.1 독립 비교 기준

Linux `cryptsetup`과 선택한 Linux Btrfs/ext4 구현을 **독립 oracle**로 사용한다. 우리 parser/IV helper로 만든 기대값만 비교하는 테스트는 호환성 증거가 아니다.

- fixture는 버전이 고정된 Linux 도구로 생성한다. OS/kernel, cryptsetup, btrfs-progs 또는 e2fsprogs, 파일시스템 타입/feature bitmask/상태/블록 크기, 이미지 geometry, cipher/KDF/sector size, 생성 명령을 manifest에 기록한다.
- 테스트 데이터와 자격증명은 실제 사용자 데이터와 분리한다. 테스트 실행 시 생성하는 임시 비밀번호도 로그에 출력하지 않는다.
- 원본 암호화 이미지 해시, Linux 복호화 볼륨 해시, 파일 manifest를 기록한다. 실제 사용자의 비밀번호·키는 수집하지 않는다.
- Linux의 평문 읽기와 Windows의 가상 장치 읽기는 같은 고정 이미지에 대해 비교한다. Linux에서 이미지 생성·정상 언마운트 후 원본을 동결한다.
- 동결한 이미지 확인은 Btrfs의 `btrfs check --readonly`, ext4의 언마운트 상태 `e2fsck -fn` 등 비수정 모드로 한정한다. ext4 파일 oracle은 clean fixture에서 read-only loop/dm-crypt 위에 `ro,noload`로 마운트하고 전후 해시를 비교한다. `ro`만으로 무쓰기를 보장하지 않는다.[17] recovery가 필요한 입력은 일관된 파일 oracle로 사용하지 않는다. 파일 복사 결과는 다른 저장장치에 쓴다. 헤더 백업만으로 데이터 백업이 된 것으로 보지 않는다.

### 10.2 요구사항과 필수 시험

의미 검증용 변조 fixture는 두 헤더 체크섬 등 앞선 검사를 통과하도록 구성한다. 예를 들어 비4000 stripes나 미지원 requirements를 시험하면서 단순 checksum 실패만 확인한 결과를 목표 검증의 성공으로 세지 않는다. 각 fixture manifest에 의도한 거부 단계와 실제 관찰 단계를 기록한다.

- **R01 / 헤더 안전성:** 정상 이중 헤더, primary/secondary 손상, seqid 차이, 잘못된 checksum·offset·UUID, 중복 JSON key, overflow, 거대한 할당 요청. 규정한 오류로 거부하고 장치를 게시하지 않는다.
- **R02 / 키 복구:** 지원 KDF/hash·XTS 키 길이의 모든 허용 조합, 올바른/잘못된 비밀번호, UTF-8 비밀번호, 선택하지 않은 슬롯, unbound/mismatched digest, 잘린 AF 영역. `Kvol=32/Kslot=64`와 그 반대의 유효 조합을 포함해 Linux 결과와 비교한다. `stripes=4000`만 허용하고 0·3999·4001·과대 값은 게시 전에 거부한다.
- **R03 / 데이터 정확성:** 전체 평문 볼륨 hash와 임의 섹터 비교, LBA 0·마지막 LBA·범위 밖·overflow·큰 요청·다중 섹터 요청. 잘못된 데이터 offset/IV 구현을 검출하는 독립 벡터를 포함한다.
- **R04 / RO:** 일반 파일 쓰기, raw disk write, UNMAP, format/partition 변경 시도를 폐기 가능한 fixture에서 실행한다. 쓰기 거부, backend write 0회, 전체 원본 hash 불변을 모두 확인한다. 커널의 WriteProtected 거부 시험과 adapter Write/Unmap 콜백 직접 호출 시험을 분리해, 앞 계층의 차단이 뒤 계층의 잘못된 구현을 가리지 않게 한다.
- **R05 / 오류:** short read, EOF, I/O 실패, 메모리 부족, KDF 제한, callback panic, 게시 실패, 중간 취소. 오류가 성공이나 영구 pending으로 바뀌지 않는다.
- **R06 / 수명주기:** 반복 open/close, 읽는 도중 정상 종료, 강제 종료, 장치 제거, 재게시, 드라이버 오류. hang·use-after-free·중복 완료·stale request를 검출한다.
- **R07 / 거부 프로파일:** 4096 sector, nonzero iv_tweak, detached header, 재암호화, integrity, OPAL, 다중 segment, 미지원 KDF/hash/required feature. 장치 게시 전에 명시적으로 거부한다.
- **R08 / Btrfs:** 고정 WinBtrfs에서 단일 장치 fixture, subvolume, 큰 파일·작은 파일·sparse file, Unicode 이름, 지원하는 compression별 파일 읽기·복사 hash를 확인한다. NTFS와 Linux 파일명·권한의 완전한 동등성은 목표가 아니다.
- **R09 / 비밀:** 모든 성공·실패·진단 경로에서 로그/argv/env에 테스트 비밀번호·키·평문이 남지 않는지 검사한다. 종료 중 메모리 제거와 라이브러리 복사본의 한계도 검토한다.
- **R10 / Windows 배포:** 실제 OS build, Secure Boot/HVCI 설정, driver signature/hash, SDK/ABI, 선택된 WinBtrfs 또는 Ext4Fsd의 인식·마운트·readonly 동작을 기록한다. 문서상의 지원을 실행 증거로 대체하지 않는다.

- **R11 / ext4 데이터:** E0의 required bits를 유지하고 64BIT/FLEX_BG/CSUM_SEED 등 optional 조합을 manifest로 고정한다. 작은/큰 파일, sparse/unwritten extent, extent 경계·다중 depth, htree directory, hardlink/symlink, UTF-8 이름을 Linux oracle과 비교한다. 전체 평문 볼륨 및 일반 파일 hash가 일치하고 복사 누락·오인식이 없어야 한다. 권한/링크의 Windows 의미 한계는 별도로 기록한다.
- **R12 / ext4 게시 전 차단:** 정상 E0와 wrong filesystem, checksum 실패, 잘린 superblock, overflow/불가능한 geometry, missing required bits, 각 미지원 feature, dirty VALID_FS/ERROR_FS, needs_recovery, orphan 상태, external journal을 독립 fixture로 시험한다. magic만 같은 ext2/ext3, 검사 결과를 다른 volume/generation에 붙인 경우도 거부한다. 의도한 오류 분류, 장치 게시 0회, 원본 hash 불변, 세션 키 정리를 확인한다. 의미 검증 fixture는 ext4 superblock checksum과 앞선 LUKS 검사를 통과하도록 만들어 목표 조건을 실제로 검증한다.
- **R13 / ext4 RO·journal:** clean E0의 최초 mount·파일 읽기·unmount·종료에서 journal wipe/replay, atime/mount/shutdown time 등 mutation 요청 0회를 확인한다. 별도 사용자 쓰기/rename/delete/UNMAP/format 요청은 거부해야 하며 callback 직접 호출과 E2E를 분리한다. dirty fixture는 실사용 mount 경로에 보내지 않는다. `ro`만 사용한 Linux oracle은 시험 구성 오류로 처리하고 read-only 하위 장치 + `ro,noload`를 사용한다. 모든 경로에서 전체 암호화 원본 hash가 같아야 한다.
- **R14 / ext4 통합 경계:** 512-byte WinSpd block과 4096-byte ext4 block의 read split/boundary, 잘못 지정한 `--filesystem`, `inspect`의 unknown_locked 출력, driver 미설치/잘못된 버전/서명/RO 미적용, 드라이버 간 간섭, Unicode·case 충돌, 링크 처리, I/O 오류와 정상/강제 종료를 시험한다. 지원하지 않는 이름을 조용히 재매핑하거나 다른 consumer로 자동 fallback하지 않는다.

### 10.3 단계별 Go / No-Go

- **G0-B: Btrfs 연결 spike.** 암호화 없는 폐기용 Btrfs 이미지로 기존 WinSpd + WinBtrfs RO 발견·마운트·정리 게이트를 유지한다.
- **G0-E: ext4 연결 spike.** 암호화 없는 E0 clean ext4 이미지로 WinSpd + Ext4Fsd RO 발견·마운트·정리·평문 해시·읽기 중 mutation 0회를 확인한다. pinned binary/서명/Secure Boot/HVCI, RO 설정 적용 순서와 512-byte device/4096-byte filesystem 조합을 기록한다. dirty 입력의 게시 거부는 실제 driver에 넘기기 전 probe 시험으로 분리한다.
- G0 두 시험은 폐기용 VM에서 수행한다. 처음에는 각 파일시스템 드라이버를 따로 시험하고, 둘을 함께 설치하는 구성은 별도 간섭 시험을 통과해야 한다. 한 consumer가 실패하면 그 경로의 통합을 중단하며 다른 consumer로 자동 대체하지 않는다.
- **G1: 독립 코어와 probe.** 공통 parser·KDF·AF·digest·512-byte XTS가 Linux oracle과 일치해야 한다. ext4 probe의 superblock/feature/clean/geometry/checksum 검증과 게시 전 차단을 추가한다. fuzzing, sanitizer, 의존성/라이선스 검토를 수행한다. LUKS 메타데이터는 비밀번호 요청 전 검증하되, 암호화된 ext4 내부 검사는 키 검증 후·장치 게시 전에 수행한다.
- **G2-B / G2-E: RO 이미지 통합.** 공통 R01–R07/R09/R10은 두 filesystem fixture 경로에 적용한다. Btrfs는 R08, ext4는 R11–R14를 추가로 통과해야 한다. 0.2.0의 Btrfs+ext4 지원 목표 완료는 **두 경로 모두**의 통과를 요구한다. 하나만 성공하면 해당 경로만 검증됨으로 보고하고 ext4 지원 완료로 표시하지 않는다. UI에서 파일이 보이는 것만으로 완료하지 않는다.
- **G3: 사용자 볼륨 호환성 판단.** 사용자 허가 후 비밀을 제외한 실제 볼륨 설정만 확인한다. LUKS 4096/KDF 또는 ext4 feature·block/inode size 등의 프로파일 불일치는 확장 설계를 먼저 검토한다. `inspect`만으로 ext4 설정을 알아냈다고 주장하지 않는다. 실제 원본 대신 별도 저장장치의 완전한 복제 이미지로 시험한다.
- **G4: 실험판 배포.** 독립 보안·스토리지 리뷰의 blocker가 없고, binaries·소스·license·SBOM·시험 로그·알려진 한계를 함께 제공한다. 서명되지 않은 개발 드라이버 시험을 일반 PC에 보안 기능 해제로 전가하지 않는다.

실제 파티션 지원은 G4 뒤의 별도 프로젝트 단계다. 읽기·쓰기 지원은 그보다 더 큰 변경이며 이 문서의 승인으로 자동 진행하지 않는다.

## 11. AI 활용 및 독립 검토 기준

GPT 6 Astra는 공통 parser, ext4 게시 전 probe, fixture harness, FFI shim, 테스트 작성과 오류 분석에 활용할 수 있다. 그러나 모델 이름을 암호학·커널 안정성의 검증 근거로 사용하지 않는다.

- 구현을 메타데이터, 키 복구, 섹터 읽기, 파일시스템 probe, WinSpd 연결, 파일시스템별 통합, 수명주기로 나누고 각각 독립 입력/출력 계약과 시험을 둔다.
- 코드 작성과 검토는 다른 컨텍스트에서 수행한다. 가능하면 Windows storage 경험자와 암호 구현 경험자의 검토를 받는다.
- 임의의 암호 구현 생성, 소스 확인 없이 WinSpd API 추정, 실제 디스크에서 바로 시험하는 작업을 금지한다.
- 필수 독립 검토 항목: 주소 단위/endian/overflow, AF·digest 연결, FFI ownership/lifetime, pending semantics, 쓰기 차단, ext4 journal/orphan/feature 거부, probe와 게시 대상 결합, 키/log 노출, 서명·라이선스.
- 발견사항은 severity와 재현 근거를 기록하고 수정 후 관련 게이트만 다시 실행한다. 실행하지 못한 Windows·보안 시험은 통과로 표시하지 않는다.

## 12. 후속 확장과 운영상 제한

### 12.1 실제 파티션 읽기 전용

장치 번호는 변경될 수 있다. 안정적인 disk/partition 식별자와 크기·partition 시작 위치를 결합하고, 열린 핸들의 대상을 재확인해야 한다. 같은 장치의 다른 쓰기 주체를 배제하고, OS volume lock/offline·hotplug·sector alignment·캐시 일관성을 검증해야 한다. 파일 이미지에서 동작했다고 이 단계가 자동으로 안전해지는 것은 아니다.

### 12.2 읽기·쓰기

별도 ADR과 데이터 손실 위험 검토가 필요하다. 최소한 partial write, flush/FUA, ordering, cancellation, power loss, 장치 제거, 재연결, 4Kn/512e, 쓰기 실패 전파, Linux↔Windows 반복 마운트 시험을 추가한다. Btrfs CoW/checksum이나 ext4 journal/metadata checksum은 잘못된 하위 저장장치의 완료 응답을 보완하지 못한다.

RW를 실험하더라도 헤더·키슬롯 변경과 online reencryption은 별도 비목표로 유지할 수 있다. RW 기능을 RO binary의 숨겨진 스위치로 제공하지 않는다.

### 12.3 라이선스와 배포

WinSpd README는 GPLv3 및 FOSS 예외, 상용 라이선스를 안내한다.[9] 구체적인 결합·배포 조건은 라이선스 전문과 선택 라이브러리의 조건을 확인해야 한다. Ext4Fsd README는 GPLv2를 안내하므로 별도 드라이버 배포·수정과 공통 라이브러리 결합의 조건을 구분해 확인한다.[13] WinBtrfs/Ext4Fsd/cryptsetup의 코드를 복사하거나 포팅하면 해당 파일의 라이선스 의무를 별도로 검토한다. Linux 코드를 읽었다는 사실과 직접 복사·링크하는 행위를 구분한다.

초기 저장소는 기존 WinBtrfs 포크가 아닌 독립 저장소로 만들 것을 권장한다. 실제 생성·구현 시 별도 작업 트리를 사용한다. 아직 저장소·빌드 파이프라인·설치 프로그램을 만들지 않았다.

## 13. 현재 근거와 미해결 항목

### 확인한 것

- WinSpd의 사용자 모드 가상 디스크 구조와 실제 header의 읽기 전용 관련 필드.[1][2][9]
- callback `TRUE/FALSE`의 완료/pending 의미와 Read/Flush 관련 설명.[3]
- LUKS2 포맷 문서와 cryptsetup의 헤더/JSON/키슬롯/IV 처리 코드.[4][5][6][7][10]
- WinBtrfs README의 per-volume `Readonly` 옵션과 적용 설명.[8]
- Ext4Fsd README, mount/RO/journal 검사 소스와 지원 feature mask. CASEFOLD 관련 README/HEAD 불일치도 확인했다.[13][14][15][16][20]
- Linux ext4의 `ro`에서도 journal replay가 발생할 수 있다는 문서와 `ro,noload`의 한계, superblock feature/state 구조.[17][18][19]

### 아직 확인하지 않은 것

- 사용자 실제 Windows 버전, Secure Boot/HVCI, 설치 가능한 WinSpd/WinBtrfs/Ext4Fsd 바이너리와 서명. Ext4Fsd 소스 HEAD와 0.71 signed release의 실제 동작 차이.
- 파티션 없는 가상 디스크에서 각 WinBtrfs/Ext4Fsd의 인식·마운트·readonly 동작과 드라이버 동시 설치 시 간섭.
- 사용자 볼륨의 섹터 크기, cipher/KDF/AF/digest, 재암호화 완료 상태, Btrfs/ext4 feature set과 ext4 clean/journal/orphan 상태.
- 최종 crypto/JSON/zeroization 라이브러리와 Windows FFI 빌드 호환성.
- shutdown/취소의 실제 pending I/O 수명과 강제 종료 후 장치 정리 동작.
- ext4 E0의 64BIT/CSUM_SEED 등 선택 조합, superblock checksum probe 구현, 이름/링크 처리, hidden metadata write 부재.
- 구현 성능, fuzzing 결과, 독립 보안 검토, 두 파일시스템의 Windows 통합 시험.

위 미확인 항목 때문에 특정 기간·성공률·MB/s·실제 디스크 안전성을 보장하지 않는다. **다음 승인 대상은 G0-B/G0-E의 폐기용 이미지 연결 시험이며 실제 원본 접근이나 RW 구현이 아니다.**

## 14. 출처와 고정 revision

조회일: 2026-09-10. 아래는 구현 사실을 확인한 소스이며, 본 문서의 제품 정책/제한과 구분한다. 확인한 소스 revision과 실제 배포 바이너리가 같다는 뜻은 아니다.

- WinSpd: `55c53bc454afbbba38bd1692f52beb77ed59142f`
- cryptsetup GitHub mirror: `ff3f1723e0e65003e35b82094642b0f0a3583ddb`
- WinBtrfs: `a0648190b4b238ebc016ce247d91ccc814a8633b`
- Ext4Fsd: `da27068c3e22f605caa5e7e52fb3535ae37031d0`
- Linux ext4 문서: `50d05c7c76c96b90462f24debacca971d2e86713`

[13]: https://github.com/bobranten/Ext4Fsd/blob/da27068c3e22f605caa5e7e52fb3535ae37031d0/README.md
[14]: https://github.com/bobranten/Ext4Fsd/blob/da27068c3e22f605caa5e7e52fb3535ae37031d0/Ext4Fsd/memory.c
[15]: https://github.com/bobranten/Ext4Fsd/blob/da27068c3e22f605caa5e7e52fb3535ae37031d0/Ext4Fsd/ext3/recover.c
[16]: https://github.com/bobranten/Ext4Fsd/blob/da27068c3e22f605caa5e7e52fb3535ae37031d0/Ext4Fsd/include/linux/ext4.h
[17]: https://github.com/torvalds/linux/blob/50d05c7c76c96b90462f24debacca971d2e86713/Documentation/admin-guide/ext4.rst
[18]: https://github.com/torvalds/linux/blob/50d05c7c76c96b90462f24debacca971d2e86713/Documentation/filesystems/ext4/super.rst
[19]: https://github.com/torvalds/linux/blob/50d05c7c76c96b90462f24debacca971d2e86713/Documentation/filesystems/ext4/journal.rst
[20]: https://github.com/bobranten/Ext4Fsd/blob/da27068c3e22f605caa5e7e52fb3535ae37031d0/Ext4Fsd/include/ext2fs.h

[1]: https://github.com/winfsp/winspd/blob/55c53bc454afbbba38bd1692f52beb77ed59142f/inc/winspd/winspd.h
[2]: https://github.com/winfsp/winspd/blob/55c53bc454afbbba38bd1692f52beb77ed59142f/inc/winspd/ioctl.h
[3]: https://github.com/winfsp/winspd/blob/55c53bc454afbbba38bd1692f52beb77ed59142f/doc/WinSpd-Tutorial.asciidoc
[4]: https://github.com/mbroz/cryptsetup/blob/ff3f1723e0e65003e35b82094642b0f0a3583ddb/lib/luks2/luks2_keyslot_luks2.c
[5]: https://github.com/mbroz/cryptsetup/blob/ff3f1723e0e65003e35b82094642b0f0a3583ddb/lib/luks2/luks2_disk_metadata.c
[6]: https://github.com/mbroz/cryptsetup/blob/ff3f1723e0e65003e35b82094642b0f0a3583ddb/lib/luks2/luks2_json_metadata.c
[7]: https://github.com/mbroz/cryptsetup/blob/ff3f1723e0e65003e35b82094642b0f0a3583ddb/docs/on-disk-format-luks2.pdf
[8]: https://github.com/maharmstone/btrfs/blob/a0648190b4b238ebc016ce247d91ccc814a8633b/README.md
[9]: https://github.com/winfsp/winspd/blob/55c53bc454afbbba38bd1692f52beb77ed59142f/README.md
[10]: https://github.com/mbroz/cryptsetup/blob/ff3f1723e0e65003e35b82094642b0f0a3583ddb/lib/crypto_backend/crypto_storage.c
[11]: https://github.com/winfsp/winspd/blob/55c53bc454afbbba38bd1692f52beb77ed59142f/src/dll/library.c
[12]: https://github.com/maharmstone/btrfs/issues/594#issuecomment-1720361600

- [1] WinSpd public API.
- [2] WinSpd storage unit geometry, flags, request/response 구조.
- [3] WinSpd tutorial: callback completion, read/write/flush/unmap.
- [4] cryptsetup LUKS2 키슬롯 처리.
- [5] cryptsetup LUKS2 디스크 메타데이터 처리.
- [6] cryptsetup LUKS2 JSON 검증과 requirements.
- [7] LUKS2 on-disk format 문서.
- [8] WinBtrfs README와 mount options.
- [9] WinSpd README: 구조·라이선스.
- [10] cryptsetup storage cipher/IV 구현.
- [11] WinSpd DLL compilation unit. 수명주기의 전체 구현 검증은 후속 게이트에 남긴다.
- [12] WinBtrfs 유지보수자의 LUKS2 계층 분리 답변.
