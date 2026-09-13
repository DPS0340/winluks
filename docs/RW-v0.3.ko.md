# v0.3 RW 확장

사용자의 RW 구현 요청에 따라 v0.2의 RO 전용 범위를 확장한다. 원본 설계 문서는 당시
사양으로 보존한다. Windows에서 지원하는 게시 경로는 현재 **Btrfs RO/RW**이며, ext4는
기존 G0 발견 실패 때문에 계속 차단한다. 독립 LUKS 코어의 암호화·복호화는 두 파일시스템
fixture로 비교한다.

## 동작 계약

- 실행 파일은 `winluks2.exe`이며 기본값은 RO다. 쓰기는 `open --read-write`로 명시한다.
  `--read-only`와 `--read-write`를 동시에 지정할 수 없다.
- RW 이미지는 기존의 로컬 일반 파일을 독점 핸들로 연다. Windows에서는 다른 읽기·쓰기·삭제
  핸들과의 공유를 거부한다. Linux의 시험용 파일 잠금은 advisory이므로 외부 도구도 잠금을
  준수해야 한다. 파일 생성·확장·축소·물리 장치·원격 파일·discard는 제공하지 않는다.
- 쓰기는 검증된 LUKS 데이터 세그먼트 안에서만 허용한다. 512바이트 정렬과 최대 1 MiB 전송을
  검사하고, 각 512바이트 데이터 단위의 AES-XTS IV로 다시 암호화한다. 헤더와 키슬롯은
  쓰기 대상이 아니다. 읽기·쓰기·flush는 같은 세션 잠금 아래에서 순서를 보장한다.
- WinSpd는 RW일 때 WriteProtected=0, 두 모드 모두 CacheSupported=0, UnmapSupported=0이다.
  Windows 파일 핸들의 write-through와 매 쓰기 후 `sync_all`을 사용하며, backing-file flush가
  성공한 뒤에만 쓰기를 완료한다. FUA가 없는 요청도 같은 계약을 따른다. 이것이 전체 파일의
  트랜잭션이나 정전 중 섹터 쓰기의 원자성을 제공하는 것은 아니다.
  파일시스템이 아직 브리지로 전달하지 않은 트랜잭션은 별개이며, 정상 종료의 볼륨 lock에서 commit한다.
  **v0.3.1 실측 제한:** 고정 WinBtrfs의 파일 `FlushFileBuffers` 성공만으로 트랜잭션 지속성을
  보장할 수 없다. 실제 강제 종료 뒤 성공 응답을 받은 새 파일이 사라졌다. 데이터베이스와
  fsync/FlushFileBuffers 보장에 의존하는 작업은 지원 범위 밖이다.
- short write는 남은 바이트를 계속 쓰고, 쓰기 또는 flush 실패는 세션을 영구 오류 상태로
  바꾼다. 이후 쓰기를 성공 처리하지 않는다. 해당 종료는 `UNCLEAN_CLOSE`로 보고한다.
- WinBtrfs v1.10과 WinSpd의 기존 바이너리를 그대로 사용한다. RW 시험 VM에서는
  `prepare-drivers.ps1 -Filesystem btrfs -AccessMode rw -TrustPinnedPublishers` 실행 후 재부팅한다.
  이 설정에서 RO 세션도 백엔드와 가상 디스크의 쓰기 방지로 동작하며 실제 파일시스템 RO
  플래그를 확인한다. per-volume Readonly override가 있으면 RW 마운트가 거부될 수 있다.

## 게시와 종료

게시 후 파일시스템 볼륨의 storage descriptor를 통해 현재 WinSpd 세션의 무작위 SCSI serial을
확인한다. 일치하는 단일 Btrfs 볼륨과 요청한 RO/RW 플래그를 확인한 뒤 `PUBLISHED_RO` 또는
`PUBLISHED_RW`를 표시한다. 드라이브 문자로 종료 대상을 추정하지 않는다.

RW에서 Ctrl+C는 볼륨 lock → filesystem flush → dismount → backing-file flush → callback drain
→ 장치 제거 순서로 종료한다. 열린 파일 등으로 lock에 실패하면 `CLOSE_BLOCKED`를 출력하고
세션을 유지한다. 파일을 닫고 Ctrl+C를 다시 누르면 재시도한다. 강제 종료, 게스트 전원 차단,
I/O 실패 시 정상 종료를 보장하지 않으며, 복제 이미지를 Linux에서 오프라인 검사해야 한다.

v0.3.1은 callback drain **이후**의 최종 오류를 판정한다. 실제 볼륨이 강제로 RO로 바뀌었거나
flush/dismount/dispatcher에서 실패하면 정상 종료로 보고하지 않는다. 재시도 가능한 lock의
사용 중 오류만 `CLOSE_BLOCKED`로 남기며, 다른 종료 오류는 `UNCLEAN_CLOSE`와 실패 exit code다.
게시 전 이미지에 존재하는 모든 Btrfs superblock mirror를 검증하고 불일치를 거부한다.
동일 UUID 복사본을 WinBtrfs가 합치는 일을 막기 위해 협력하는 v0.3.1 publisher를 시스템당
하나로 제한한다. 구버전이나 다른 게시 도구와의 동시 실행은 이 잠금에 포함되지 않는다.

이 순서는 Microsoft의 [volume lock](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_lock_volume),
[dismount](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ni-winioctl-fsctl_dismount_volume)
계약 및 고정 WinBtrfs 소스의 `lock_volume`/`dismount_volume` 처리를 따른다.

## 검증 항목

1. 기존 RO 30-case oracle 회귀와 parser/probe 시험.
2. RW 30-case oracle: 첫/끝 섹터, 다중 섹터, 겹치는 쓰기, 1 MiB 쓰기 후 Linux dm-crypt의
   전체 평문을 독립 Linux 원본 + 예상 변경과 비교. LUKS 헤더/키슬롯과 원본 fixture 불변.
3. Windows 생성·비정렬 파일 덮어쓰기·append·truncate·Unicode·sparse·복사·이름 변경·삭제.
4. 열린 파일 상태의 종료 거부, 핸들 해제 후 정상 종료, Windows 재마운트 후 파일 해시.
5. 분리된 이미지를 Linux `btrfs check --readonly` 및 RO 마운트로 검사하고, 파일 내용과
   sparse 할당을 검증. Linux가 새 파일을 기록한 이미지를 Windows에서 다시 읽는다.
6. RW 드라이버 설정에서 RO 세션의 쓰기 거부, RW 세션의 백엔드 공유 거부, ext4 게시 차단.

실행 여부와 결과는 [검증 기록](VALIDATION.md), [v0.3.1 장애·정전 시험](POWERLOSS-v0.3.1.md) 및
`docs/evidence/`에 기록한다. 두 별도 AI 리뷰어의 검토와 한정된 VM 시험을 수행했으며, 외부
전문가 감사·실물 호스트/스토리지 전원 차단·모든 가능한 장애 시점의 검증을 뜻하지 않는다.
