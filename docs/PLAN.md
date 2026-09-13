# winluks 구현 계획

기준: [Windows LUKS2 Read-only Bridge 설계 v0.2.0](design-v0.2.0.ko.md).
원본 설계의 당시 작업 범위와 승인 문구는 기록이다. 현재 작업은 사용자가 요청한 구현,
가상 볼륨 기반 시험 환경, DPS0340/winluks 공개 저장소 게시를 포함한다.

이후 사용자의 RW 확장 요청을 v0.3에 반영했다. [RW 동작 계약과 계획](RW-v0.3.ko.md)은
v0.2 문서의 RO 전용 범위를 확장하며, 원본 설계는 당시 사양으로 보존한다.

v0.3.1은 사용자가 요청한 정전 시험, 별도 AI 리뷰어의 독립 검토, 발견된 결함 보완과
실험판 릴리즈를 포함한다. [시험 범위·실측 결과·남은 제한](POWERLOSS-v0.3.1.md)을 별도로
기록한다. 파일 flush 성공 후의 데이터 유실을 확인했으므로 일반적인 fsync 지속성은 약속하지 않는다.

## 목표와 경계

Windows 11 x64에서 로컬 LUKS2 파티션 이미지의 파일을 읽고 복사하며, 명시적 RW 모드로 수정한다.
Rust의 공통 암호화·복호화 코어, 최소 WinSpd C shim, 기존 파일시스템 드라이버를 사용한다.
현재 Btrfs의 RO/RW를 지원한다. 실제 장치, 자동 복구, 4096바이트 암호화 섹터는 구현하지 않는다.
이미지·VM·개발용 자격증명·덤프는 Git에 올리지 않는다.

## 개발 환경

- Linux 호스트: 편집, Git, Rust 단위 시험, fuzzing, VM 자동화.
- Linux VM: 고정 Debian 이미지, cryptsetup/btrfs-progs/e2fsprogs로 fixture와 독립 oracle 생성.
- Windows VM: 로컬 NTFS의 이미지, MSVC 빌드, WinSpd와 파일시스템 드라이버 시험.
- Windows 기본 이미지에서 Btrfs/ext4 시험 환경을 각각 복제하고 간섭 시험은 나중에 진행한다.
- 모든 저장장치는 가상 파일이다. 공유 경로는 전달에만 사용하고 fixture는 로컬로 복사한다.
- 동시 writable disk 공유는 하지 않는다. 정상 언마운트 후 동결한 동일 바이트를 비교한다.
- VM 복원 전 게스트 내부 fixture 해시와 요청 계측을 수집한다. base qcow2 해시만으로 RO를 판단하지 않는다.

## 구현 순서와 완료 증거

| 단계 | 작업 | 통과 조건 | 상태 |
|---|---|---|---|
| 준비 | 저장소, 빌드, 원본 설계, VM과 결과 경로 | 재현 가능한 명령과 버전 기록 | 공개 저장소·Linux/Windows VM·Windows CI 빌드 완료 |
| G0-B | 평문 Btrfs → WinSpd → WinBtrfs | 발견·RO 마운트·파일 해시·종료 | Secure Boot off / HVCI off VM에서 통과 |
| G0-E | 평문 E0 ext4 → WinSpd → Ext4Fsd | G0-B 항목 + RO 차단 전 mutation 관찰 | No-Go: 드라이버 실행 중에도 파티션 없는 디스크에서 볼륨 미발견; 상위 mutation 추적 미실행 |
| G1 | 이중 헤더/JSON, KDF, AF, digest, XTS, fs-probe | Linux oracle 일치, 음성 시험, fuzzing | 코어 구현; 30 fixture 비교·회귀 시험·짧은 ASan fuzz 통과, 전체 리뷰 미완료 |
| G2-B | LUKS2 Btrfs 통합 | R01–R10의 해당 항목 | CLI 실제 마운트·복사·쓰기 거부·정상 종료 통과; 전체 오류 행렬 미완료 |
| G2-E | LUKS2 ext4 통합 | 공통 항목 및 R11–R14 | G0-E 실패로 게시 차단 (`FS_GATE_UNPASSED`); 복호화·probe oracle만 검증 |
| G3 | 사용자 볼륨 호환성 | 별도 허가 후 복제 이미지 검사 | 이번 가상 fixture 작업 밖 |
| G4 | 실험판 배포 | 독립 리뷰, 게이트, SBOM, 라이선스, 실행 증거 | v0.3.1 별도 AI 리뷰·의존성 목록·대응 소스·실측 결과 포함; 외부 전문가 감사 및 실사용 안전성 인증은 미완료 |
| v0.3 RW 코어 | 독점 파일, 암호화 쓰기, flush, 오류 상태 | 독립 dm-crypt 전체 비교·범위/잠금/RO 회귀 | RW 30-case 및 RO 30-case 통과 |
| v0.3 Btrfs RW | 파일 쓰기·정상 분리·재마운트·OS 왕복 | Windows 파일 작업, busy close, Linux 검사·왕복 | 가상 이미지에서 통과; 전체 장애/정전 행렬과 독립 리뷰는 미완료 |
| v0.3.1 보완 | 전체 mirror gate·drain 후 오류·강제 RO·panic·동시 게시 | 재현 회귀, Windows native 경계 시험, 두 AI 리뷰어 재검토 | 구현 및 재검토 완료; 과거 v0.3 상태와 구분 |
| v0.3.1 장애 시험 | QEMU 강제 종료·프로세스 종료·용량 부족·정상 종료·raw block flush | 신선한 VM overlay, 재부팅 전 Linux RO 검사, 파일/전체 블록 oracle | 버전별 시험 보고서에 실행 결과와 실패를 함께 기록; 호스트/컨트롤러 정전 및 torn-sector 미실행 |

G0 실패는 해당 파일시스템의 통합을 중단하는 근거다. 독립 코어 구현과 fixture 개발은
계속할 수 있지만, 성공하지 않은 드라이버 조합을 지원 완료로 표시하지 않는다.
ext4 재개에는 발견 문제에 대한 설계 결정과 G0-E 재시험이 먼저 필요하다.
합성 GPT를 자동 추가하거나 드라이버를 패치해 현재 게이트를 우회하지 않는다.

## 모듈과 검증 항목

1. `image`: 일반 파일의 RO 또는 독점 RW 핸들, 크기 고정, 장치/원격/reparse 거부, 범위 검사.
2. `metadata`: 두 헤더 checksum/seqid/UUID/의미 비교, 중복 JSON 키, overflow, 영역 overlap.
3. `crypto`: OpenSSL AES-XTS/PBKDF2/SHA, RustCrypto Argon2, AF merge, 상수 시간 digest 비교.
4. `volume`: 512바이트 단위 IV, 정렬·경계·short I/O, 데이터 영역 쓰기·flush·오류 상태, 세션 키 제거.
5. `probe`: Btrfs 단일 장치/feature 정책 및 ext4 E0/checksum/clean/journal/orphan 검사.
6. `adapter`: WinSpd 모드별 geometry/쓰기 정책, 동기 완료, Unmap 거부, panic 격리, 볼륨 lock/dismount와 shutdown/drain.
7. `cli`: inspect/open, 기본 RO·명시적 RW, 비표시 콘솔 입력, filesystem 명시, 비밀 없는 오류/상태.
8. 시험: cryptsetup fixture, 독립 평문·파일 hash, 의미 검증용 재체크섬 변조, G0/E2E PowerShell.

## 결과 기록 원칙

실제 명령·OS build·도구 버전·바이너리 hash와 결과를 `docs/VALIDATION.md`에 기록한다.
구현됨, 컴파일됨, 단위 시험 통과, Windows 실측 통과, 독립 감사 완료를 구분한다.
Windows callback 0회는 ext4 hidden write 0회의 증거가 아니다. 필요한 상위 경계 추적을
실행하지 못했다면 G0-E/R13은 미통과로 남긴다.
