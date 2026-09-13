/* SPDX-License-Identifier: GPL-3.0-or-later
 * Independent regression tests for the real native shim. No WinSpd DLL/driver,
 * volume, service, registry setting, or named kernel object is created by tests.
 * Build from an x64 MSVC developer shell, after preparing the pinned SDK:
 *   cl /nologo /W4 /I "%WINSPD_SDK%\inc" native\tests\close.c /Fe:native-close-test.exe /link advapi32.lib ole32.lib
 *   native-close-test.exe
 */
#include <winspd/winspd.h>
#include <objbase.h>
#include <stdint.h>
#include <stddef.h>
#include <wchar.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <winioctl.h>

static HANDLE WINAPI mock_CreateMutexW(LPSECURITY_ATTRIBUTES, BOOL, LPCWSTR);
static HRESULT WINAPI mock_CoCreateGuid(GUID *);
static HANDLE WINAPI mock_FindFirstVolumeW(LPWSTR, DWORD);
static BOOL WINAPI mock_FindNextVolumeW(HANDLE, LPWSTR, DWORD);
static BOOL WINAPI mock_FindVolumeClose(HANDLE);
static HANDLE WINAPI mock_CreateFileW(LPCWSTR, DWORD, DWORD, LPSECURITY_ATTRIBUTES,
    DWORD, DWORD, HANDLE);
static BOOL WINAPI mock_DeviceIoControl(HANDLE, DWORD, LPVOID, DWORD, LPVOID,
    DWORD, LPDWORD, LPOVERLAPPED);
static BOOL WINAPI mock_GetVolumeInformationByHandleW(HANDLE, LPWSTR, DWORD,
    LPDWORD, LPDWORD, LPDWORD, LPWSTR, DWORD);
static BOOL WINAPI mock_FlushFileBuffers(HANDLE);
static BOOL WINAPI mock_CloseHandle(HANDLE);
static DWORD mock_SpdStorageUnitCreate(PWSTR, const SPD_STORAGE_UNIT_PARAMS *,
    const SPD_STORAGE_UNIT_INTERFACE *, SPD_STORAGE_UNIT **);
static DWORD mock_SpdStorageUnitStartDispatcher(SPD_STORAGE_UNIT *, ULONG);
static VOID mock_SpdStorageUnitGetDispatcherError(SPD_STORAGE_UNIT *, DWORD *);
static VOID mock_SpdStorageUnitShutdown(SPD_STORAGE_UNIT *);
static VOID mock_SpdStorageUnitWaitDispatcher(SPD_STORAGE_UNIT *);
static VOID mock_SpdStorageUnitDelete(SPD_STORAGE_UNIT *);

/* Parse the platform declarations first. Only this translation unit substitutes
 * the platform boundary; the production source itself is included unmodified. */
#define CreateMutexW mock_CreateMutexW
#define CoCreateGuid mock_CoCreateGuid
#define FindFirstVolumeW mock_FindFirstVolumeW
#define FindNextVolumeW mock_FindNextVolumeW
#define FindVolumeClose mock_FindVolumeClose
#define CreateFileW mock_CreateFileW
#define DeviceIoControl mock_DeviceIoControl
#define GetVolumeInformationByHandleW mock_GetVolumeInformationByHandleW
#define FlushFileBuffers mock_FlushFileBuffers
#define CloseHandle mock_CloseHandle
#define SpdStorageUnitCreate mock_SpdStorageUnitCreate
#define SpdStorageUnitStartDispatcher mock_SpdStorageUnitStartDispatcher
#define SpdStorageUnitGetDispatcherError mock_SpdStorageUnitGetDispatcherError
#define SpdStorageUnitShutdown mock_SpdStorageUnitShutdown
#define SpdStorageUnitWaitDispatcher mock_SpdStorageUnitWaitDispatcher
#define SpdStorageUnitDelete mock_SpdStorageUnitDelete
#include "../winspd_shim.c"

#define CHECK(expr) do { if (!(expr)) { \
    fprintf(stderr, "%s:%d: %s\n", __FILE__, __LINE__, #expr); exit(1); \
} } while (0)
#define VOLUME ((HANDLE)(uintptr_t)0x1110)
#define SEARCH ((HANDLE)(uintptr_t)0x2220)
#define GUARD ((HANDLE)(uintptr_t)0x3330)
static const wchar_t volume_name[] = L"\\\\?\\Volume{11223344-5566-7788-90ab-cdef01234567}\\";
static const char serial_name[] = "11223344-5566-7788-90ab-cdef01234567";
static const GUID session_guid = {0x11223344, 0x5566, 0x7788,
    {0x90, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67}};
static SPD_STORAGE_UNIT unit;
static struct {
    DWORD before_error, after_error, mutex_error, create_error, start_error;
    DWORD lock_error, flush_error, dismount_error;
    DWORD mode_error[2], mode_flags[2];
    unsigned mode_calls, lock_calls, flush_calls, dismount_calls, unlock_calls;
    unsigned volume_closes, guard_closes, creates, starts, mutex_calls;
    unsigned shutdowns, waits, deletes, error_queries, find_calls;
    int locked, descriptor_matches;
    char teardown[16];
    unsigned teardown_len;
} state;
static unsigned cases;

static void reset(void) {
    memset(&state, 0, sizeof state);
    memset(&unit, 0, sizeof unit);
    state.descriptor_matches = 1;
    SetLastError(ERROR_SUCCESS);
}
static void event(char code) {
    CHECK(state.teardown_len + 1 < sizeof state.teardown);
    state.teardown[state.teardown_len++] = code;
    state.teardown[state.teardown_len] = 0;
}
static WL_SESSION *session(void) {
    WL_SESSION *s = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof *s);
    CHECK(s != NULL);
    s->unit = &unit;
    s->publish_guard = GUARD;
    memcpy(s->serial, serial_name, sizeof serial_name);
    unit.UserContext = s;
    return s;
}
static BOOL fail(DWORD error) {
    SetLastError(error);
    return error == ERROR_SUCCESS;
}

static HANDLE WINAPI mock_CreateMutexW(LPSECURITY_ATTRIBUTES security, BOOL owner, LPCWSTR name) {
    CHECK(!security && !owner);
    CHECK(!wcscmp(name, L"Global\\winluks-publication-v1"));
    state.mutex_calls++;
    SetLastError(state.mutex_error);
    return state.mutex_error && state.mutex_error != ERROR_ALREADY_EXISTS ? NULL : GUARD;
}
static HRESULT WINAPI mock_CoCreateGuid(GUID *guid) { *guid = session_guid; return S_OK; }
static HANDLE WINAPI mock_FindFirstVolumeW(LPWSTR name, DWORD capacity) {
    CHECK(capacity >= sizeof volume_name / sizeof *volume_name);
    wcscpy_s(name, capacity, volume_name);
    state.find_calls++;
    return SEARCH;
}
static BOOL WINAPI mock_FindNextVolumeW(HANDLE search, LPWSTR name, DWORD capacity) {
    CHECK(search == SEARCH); (void)name; (void)capacity;
    return fail(ERROR_NO_MORE_FILES);
}
static BOOL WINAPI mock_FindVolumeClose(HANDLE search) { CHECK(search == SEARCH); return TRUE; }
static HANDLE WINAPI mock_CreateFileW(LPCWSTR name, DWORD access, DWORD sharing,
    LPSECURITY_ATTRIBUTES security, DWORD disposition, DWORD flags, HANDLE template_file) {
    size_t n = wcslen(volume_name) - 1;
    CHECK(wcslen(name) == n && !wcsncmp(name, volume_name, n));
    CHECK(access == (GENERIC_READ | GENERIC_WRITE));
    CHECK(sharing == (FILE_SHARE_READ | FILE_SHARE_WRITE));
    CHECK(!security && disposition == OPEN_EXISTING && !flags && !template_file);
    return VOLUME;
}
static BOOL WINAPI mock_DeviceIoControl(HANDLE volume, DWORD code, LPVOID in, DWORD in_size,
    LPVOID out, DWORD out_size, LPDWORD returned, LPOVERLAPPED overlapped) {
    CHECK(volume == VOLUME && !overlapped && returned);
    *returned = 0;
    if (code == IOCTL_STORAGE_QUERY_PROPERTY) {
        STORAGE_DEVICE_DESCRIPTOR *d = out;
        STORAGE_PROPERTY_QUERY *query = in;
        DWORD size = (DWORD)(sizeof *d + sizeof serial_name);
        CHECK(in_size == sizeof *query && query->PropertyId == StorageDeviceProperty);
        CHECK(query->QueryType == PropertyStandardQuery && out_size >= size);
        memset(out, 0, size);
        d->Size = size; d->Version = sizeof *d; d->SerialNumberOffset = sizeof *d;
        memcpy((BYTE *)out + sizeof *d, serial_name, sizeof serial_name);
        if (!state.descriptor_matches) ((BYTE *)out)[sizeof *d] = '0';
        *returned = size;
        return TRUE;
    }
    CHECK(!in && !in_size && !out && !out_size);
    switch (code) {
    case FSCTL_LOCK_VOLUME:
        state.lock_calls++;
        if (!state.lock_error) state.locked = 1;
        return fail(state.lock_error);
    case FSCTL_UNLOCK_VOLUME:
        CHECK(state.locked);
        state.unlock_calls++; state.locked = 0;
        return TRUE;
    case FSCTL_DISMOUNT_VOLUME:
        CHECK(state.locked && state.flush_calls == 1 && !state.flush_error);
        state.dismount_calls++;
        return fail(state.dismount_error);
    default: CHECK(0); return FALSE;
    }
}
static BOOL WINAPI mock_GetVolumeInformationByHandleW(HANDLE volume, LPWSTR label,
    DWORD label_size, LPDWORD serial, LPDWORD max, LPDWORD flags, LPWSTR fs, DWORD capacity) {
    unsigned call = state.mode_calls++;
    CHECK(volume == VOLUME && call < 2 && !label && !label_size && capacity >= 6);
    if (state.mode_error[call]) return fail(state.mode_error[call]);
    *serial = 1; *max = 255; *flags = state.mode_flags[call];
    wcscpy_s(fs, capacity, L"Btrfs");
    return fail(ERROR_SUCCESS); /* May clobber the prior lock error: callers must save it. */
}
static BOOL WINAPI mock_FlushFileBuffers(HANDLE volume) {
    CHECK(volume == VOLUME && state.locked && state.mode_calls == 2);
    state.flush_calls++;
    return fail(state.flush_error);
}
static BOOL WINAPI mock_CloseHandle(HANDLE handle) {
    if (handle == VOLUME) {
        state.volume_closes++; state.locked = 0;
        if (state.shutdowns) { CHECK(state.deletes == 1); event('v'); }
    } else {
        CHECK(handle == GUARD); state.guard_closes++;
        if (state.shutdowns) { CHECK(state.deletes == 1); event('g'); }
    }
    return TRUE;
}
static DWORD mock_SpdStorageUnitCreate(PWSTR name, const SPD_STORAGE_UNIT_PARAMS *params,
    const SPD_STORAGE_UNIT_INTERFACE *callbacks, SPD_STORAGE_UNIT **out) {
    CHECK(!name && params && callbacks && out);
    state.creates++;
    if (state.create_error) return state.create_error;
    unit.StorageUnitParams = *params; unit.Interface = callbacks;
    *out = &unit;
    return ERROR_SUCCESS;
}
static DWORD mock_SpdStorageUnitStartDispatcher(SPD_STORAGE_UNIT *u, ULONG count) {
    CHECK(u == &unit && count == 1); state.starts++;
    return state.start_error;
}
static VOID mock_SpdStorageUnitGetDispatcherError(SPD_STORAGE_UNIT *u, DWORD *error) {
    CHECK(u == &unit && !state.deletes);
    state.error_queries++;
    if (state.waits) { event('a'); *error = state.after_error; }
    else { event('b'); *error = state.before_error; }
}
static VOID mock_SpdStorageUnitShutdown(SPD_STORAGE_UNIT *u) {
    CHECK(u == &unit && !state.shutdowns++); event('s');
}
static VOID mock_SpdStorageUnitWaitDispatcher(SPD_STORAGE_UNIT *u) {
    CHECK(u == &unit && state.shutdowns == 1 && !state.waits++); event('w');
}
static VOID mock_SpdStorageUnitDelete(SPD_STORAGE_UNIT *u) {
    CHECK(u == &unit && state.waits == 1 && !state.deletes++); event('d');
}

static void drain_cases(void) {
    const DWORD errors[][3] = {
        {0, 0, 0}, {0, ERROR_OPERATION_ABORTED, 0},
        {0, ERROR_IO_DEVICE, ERROR_IO_DEVICE}, {0, ERROR_INVALID_HANDLE, ERROR_INVALID_HANDLE},
        {ERROR_ACCESS_DENIED, ERROR_OPERATION_ABORTED, ERROR_ACCESS_DENIED},
        {ERROR_OPERATION_ABORTED, ERROR_OPERATION_ABORTED, ERROR_OPERATION_ABORTED}
    };
    unsigned i;
    for (i = 0; i < sizeof errors / sizeof *errors; i++) {
        WL_SESSION *s;
        reset(); s = session(); s->locked_volume = VOLUME; state.locked = 1;
        state.before_error = errors[i][0]; state.after_error = errors[i][1];
        CHECK(wl_close(s) == errors[i][2]);
        CHECK(!strcmp(state.teardown, "bswadvg"));
        CHECK(state.error_queries == 2 && state.volume_closes == 1 && state.guard_closes == 1);
        CHECK(!state.locked);
        cases++;
    }
}
static void expect_close(DWORD error, DWORD expected_phase, unsigned locks,
    unsigned modes, unsigned flushes, unsigned dismounts, unsigned unlocks) {
    WL_SESSION *s = session();
    DWORD phase = 99;
    CHECK(wl_lock_and_dismount(s, &phase) == error && phase == expected_phase);
    CHECK(state.lock_calls == locks && state.mode_calls == modes && state.flush_calls == flushes);
    CHECK(state.dismount_calls == dismounts && state.unlock_calls == unlocks);
    if (error) {
        CHECK(!s->locked_volume && state.volume_closes == 1 && !state.locked);
    } else {
        unsigned found = state.find_calls;
        CHECK(s->locked_volume == VOLUME && state.locked && !state.volume_closes);
        CHECK(wl_lock_and_dismount(s, &phase) == ERROR_SUCCESS); /* Idempotent, retain lock. */
        CHECK(state.find_calls == found && !state.volume_closes);
    }
    CHECK(wl_close(s) == ERROR_SUCCESS);
    CHECK(state.volume_closes == 1 && state.guard_closes == 1 && !state.locked);
    cases++;
}
static void close_phase_cases(void) {
    const DWORD busy[] = {ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION, ERROR_BUSY};
    unsigned i;
    for (i = 0; i < sizeof busy / sizeof *busy; i++) {
        reset(); state.lock_error = busy[i];
        expect_close(busy[i], 1, 1, 2, 0, 0, 0);
    }
    reset(); state.lock_error = ERROR_IO_DEVICE;
    expect_close(ERROR_IO_DEVICE, 1, 1, 2, 0, 0, 0);
    reset(); state.mode_flags[0] = FILE_READ_ONLY_VOLUME;
    expect_close(ERROR_WRITE_PROTECT, 0, 0, 1, 0, 0, 0);
    reset(); state.lock_error = ERROR_ACCESS_DENIED; state.mode_flags[1] = FILE_READ_ONLY_VOLUME;
    expect_close(ERROR_WRITE_PROTECT, 0, 1, 2, 0, 0, 0);
    reset(); state.mode_flags[1] = FILE_READ_ONLY_VOLUME;
    expect_close(ERROR_WRITE_PROTECT, 2, 1, 2, 0, 0, 1);
    reset(); state.mode_error[0] = ERROR_IO_DEVICE;
    expect_close(ERROR_IO_DEVICE, 0, 0, 1, 0, 0, 0);
    reset(); state.mode_error[1] = ERROR_INVALID_FUNCTION;
    expect_close(ERROR_INVALID_FUNCTION, 2, 1, 2, 0, 0, 1);
    reset(); state.flush_error = ERROR_ACCESS_DENIED;
    expect_close(ERROR_ACCESS_DENIED, 2, 1, 2, 1, 0, 1);
    reset(); state.dismount_error = ERROR_ACCESS_DENIED;
    expect_close(ERROR_ACCESS_DENIED, 3, 1, 2, 1, 1, 1);
    reset(); state.descriptor_matches = 0;
    expect_close(ERROR_NOT_READY, 0, 0, 0, 0, 0, 0);
    reset(); expect_close(ERROR_SUCCESS, 4, 1, 2, 1, 1, 0);
}
static int no_read(void *context, uint64_t lba, uint32_t count, void *buffer) {
    (void)context; (void)lba; (void)count; (void)buffer; CHECK(0); return 1;
}
static int no_write(void *context, uint64_t lba, uint32_t count, const void *buffer) {
    (void)context; (void)lba; (void)count; (void)buffer; CHECK(0); return 1;
}
static int no_control(void *context, uint32_t op, uint64_t lba, uint32_t count) {
    (void)context; (void)op; (void)lba; (void)count; CHECK(0); return 1;
}
static void publication_cases(void) {
    WL_SESSION *s;
    reset(); state.mutex_error = ERROR_ALREADY_EXISTS;
    CHECK(wl_create(&unit, no_read, no_write, no_control, 8, 0, &s) == ERROR_BUSY);
    CHECK(!s && state.mutex_calls == 1 && !state.creates && !state.starts && state.guard_closes == 1);
    cases++;
    reset(); state.mutex_error = ERROR_ACCESS_DENIED;
    CHECK(wl_create(&unit, no_read, no_write, no_control, 8, 0, &s) == ERROR_ACCESS_DENIED);
    CHECK(!s && !state.creates && !state.guard_closes);
    cases++;
    reset(); state.create_error = ERROR_IO_DEVICE;
    CHECK(wl_create(&unit, no_read, no_write, no_control, 8, 0, &s) == ERROR_IO_DEVICE);
    CHECK(!s && state.creates == 1 && !state.starts && state.guard_closes == 1);
    cases++;
    reset(); state.start_error = ERROR_NOT_ENOUGH_MEMORY;
    CHECK(wl_create(&unit, no_read, no_write, no_control, 8, 0, &s) == ERROR_NOT_ENOUGH_MEMORY);
    CHECK(!s && state.starts == 1 && state.guard_closes == 1);
    CHECK(!strcmp(state.teardown, "swdg"));
    cases++;
    reset(); CHECK(wl_create(&unit, no_read, no_write, no_control, 8, 0, &s) == ERROR_SUCCESS);
    CHECK(s && state.creates == 1 && state.starts == 1 && !state.guard_closes);
    CHECK(!strcmp(s->serial, serial_name));
    CHECK(wl_close(s) == ERROR_SUCCESS && state.guard_closes == 1);
    cases++;
}
int main(void) {
    drain_cases();
    close_phase_cases();
    publication_cases();
    printf("native lifecycle tests: %u cases passed (mocked OS/WinSpd boundary)\n", cases);
    return 0;
}
