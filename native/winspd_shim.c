/* SPDX-License-Identifier: GPL-3.0-or-later */
/* ABI verified against WinSpd 55c53bc454afbbba38bd1692f52beb77ed59142f. */
#include <winspd/winspd.h>
#include <objbase.h>
#include <stdint.h>
#include <stddef.h>
#include <wchar.h>
#include <stdio.h>
#include <winioctl.h>

typedef char assert_params_size[(sizeof(SPD_STORAGE_UNIT_PARAMS) == 128) ? 1 : -1];
typedef char assert_status_size[(sizeof(SPD_STORAGE_UNIT_STATUS) == 32) ? 1 : -1];
typedef char assert_interface_size[(sizeof(SPD_STORAGE_UNIT_INTERFACE) == 16 * sizeof(void *)) ? 1 : -1];

typedef int (*WL_READ)(void *, uint64_t, uint32_t, void *);
typedef int (*WL_WRITE)(void *, uint64_t, uint32_t, const void *);
typedef int (*WL_CONTROL)(void *, uint32_t, uint64_t, uint32_t);
typedef struct {
    SPD_STORAGE_UNIT *unit;
    void *context;
    WL_READ read;
    WL_WRITE write;
    WL_CONTROL control;
    HANDLE locked_volume;
    HANDLE publish_guard;
    char serial[37];
} WL_SESSION;

static void result(SPD_STORAGE_UNIT_STATUS *s, int code) {
    memset(s, 0, sizeof *s);
    if (!code) return;
    s->ScsiStatus = 2; /* CHECK CONDITION */
    switch (code) {
        case 1: s->SenseKey = 7; s->ASC = 0x27; break; /* DATA PROTECT */
        case 2: s->SenseKey = 5; s->ASC = 0x21; break; /* LBA OUT OF RANGE */
        case 3: s->SenseKey = 2; s->ASC = 0x04; break; /* NOT READY */
        case 5: s->SenseKey = 3; s->ASC = 0x0c; break; /* WRITE ERROR */
        case 6: s->SenseKey = 5; s->ASC = 0x20; break; /* UNSUPPORTED COMMAND */
        default: s->SenseKey = 3; s->ASC = 0x11; break; /* UNRECOVERED READ */
    }
}
static BOOLEAN read_cb(SPD_STORAGE_UNIT *u, void *buffer, UINT64 lba, UINT32 count,
    BOOLEAN fua, SPD_STORAGE_UNIT_STATUS *status) {
    WL_SESSION *s = u->UserContext;
    (void)fua; /* Every acknowledged write has already reached the backing-file flush. */
    if (count > 2048 || (count && !buffer)) { result(status, 2); return TRUE; }
    result(status, s->read(s->context, lba, count, buffer));
    /* TRUE means completed; it does not mean successful. */
    return TRUE;
}
static BOOLEAN write_cb(SPD_STORAGE_UNIT *u, void *buffer, UINT64 lba, UINT32 count,
    BOOLEAN fua, SPD_STORAGE_UNIT_STATUS *status) {
    WL_SESSION *s = u->UserContext;
    (void)fua; /* RW is write-through even when FUA is clear. */
    if (count > 2048 || (count && !buffer)) { result(status, 2); return TRUE; }
    result(status, s->write(s->context, lba, count, buffer)); return TRUE;
}
static BOOLEAN flush_cb(SPD_STORAGE_UNIT *u, UINT64 lba, UINT32 count,
    SPD_STORAGE_UNIT_STATUS *status) {
    WL_SESSION *s = u->UserContext;
    result(status, s->control(s->context, 2, lba, count)); return TRUE;
}
static BOOLEAN unmap_cb(SPD_STORAGE_UNIT *u, SPD_UNMAP_DESCRIPTOR *d, UINT32 count,
    SPD_STORAGE_UNIT_STATUS *status) {
    WL_SESSION *s = u->UserContext;
    (void)d;
    result(status, s->control(s->context, 3, 0, count)); return TRUE;
}
static const SPD_STORAGE_UNIT_INTERFACE iface = {read_cb, write_cb, flush_cb, unmap_cb, {0}};

/* Read-only preflight: never starts services or changes registry policy. */
static int running(const wchar_t *name) {
    SC_HANDLE manager = OpenSCManagerW(0, 0, SC_MANAGER_CONNECT), service;
    SERVICE_STATUS status;
    int ok = 0;
    if (!manager) return 0;
    service = OpenServiceW(manager, name, SERVICE_QUERY_STATUS);
    if (service) {
        ok = QueryServiceStatus(service, &status) && status.dwCurrentState == SERVICE_RUNNING;
        CloseServiceHandle(service);
    }
    CloseServiceHandle(manager);
    return ok;
}
static int setting(const wchar_t *path, const wchar_t *name, DWORD expected) {
    DWORD value = ~expected, size = sizeof value;
    return ERROR_SUCCESS == RegGetValueW(HKEY_LOCAL_MACHINE, path, name,
        RRF_RT_REG_DWORD, 0, &value, &size) && value == expected;
}
DWORD wl_consumer_ready(uint32_t filesystem, uint32_t read_only, wchar_t *driver, uint32_t capacity) {
    const wchar_t *path, *file;
    UINT n;
    if (!driver || capacity < MAX_PATH) return ERROR_INVALID_PARAMETER;
    if (filesystem == 0) {
        path = L"SYSTEM\\CurrentControlSet\\Services\\btrfs";
        file = L"\\drivers\\btrfs.sys";
        if (!running(L"btrfs") || !(setting(path, L"Readonly", 0) ||
            (read_only && setting(path, L"Readonly", 1)))) return ERROR_NOT_READY;
    } else if (filesystem == 1) {
        if (!read_only) return ERROR_NOT_SUPPORTED;
        path = L"SYSTEM\\CurrentControlSet\\Services\\Ext2Fsd\\Parameters";
        file = L"\\drivers\\Ext2Fsd.sys";
        if (!running(L"Ext2Fsd") || !setting(path, L"WritingSupport", 0) ||
            !setting(path, L"Ext3ForceWriting", 0) || !setting(path, L"Readonly", 1))
            return ERROR_NOT_READY;
    } else return ERROR_INVALID_PARAMETER;
    n = GetSystemDirectoryW(driver, capacity);
    if (!n || n + wcslen(file) >= capacity) return ERROR_INSUFFICIENT_BUFFER;
    wcscat_s(driver, capacity, file);
    return ERROR_SUCCESS;
}

DWORD wl_create(void *context, WL_READ read, WL_WRITE write, WL_CONTROL control, uint64_t blocks,
    uint32_t read_only, WL_SESSION **out) {
    SPD_STORAGE_UNIT_PARAMS p = {0};
    WL_SESSION *s;
    DWORD error;
    *out = 0;
    if (!blocks || !context || !read || !write || !control) return ERROR_INVALID_PARAMETER;
    s = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof *s);
    if (!s) return ERROR_NOT_ENOUGH_MEMORY;
    /* Same-UUID image copies must not be mounted concurrently by this bridge. */
    s->publish_guard = CreateMutexW(0, FALSE, L"Global\\winluks-publication-v1");
    error = GetLastError();
    if (!s->publish_guard || error == ERROR_ALREADY_EXISTS) {
        if (s->publish_guard) CloseHandle(s->publish_guard);
        HeapFree(GetProcessHeap(), 0, s);
        return error == ERROR_ALREADY_EXISTS ? ERROR_BUSY : error;
    }
    s->context = context; s->read = read; s->write = write; s->control = control;
    if (FAILED(CoCreateGuid(&p.Guid))) { CloseHandle(s->publish_guard); HeapFree(GetProcessHeap(), 0, s); return ERROR_GEN_FAILURE; }
    /* WinSpd's pinned kernel formats the SCSI serial from this session GUID. */
    sprintf_s(s->serial, sizeof s->serial, "%08lx-%04x-%04x-%02x%02x-%02x%02x%02x%02x%02x%02x",
        p.Guid.Data1, p.Guid.Data2, p.Guid.Data3, p.Guid.Data4[0], p.Guid.Data4[1],
        p.Guid.Data4[2], p.Guid.Data4[3], p.Guid.Data4[4], p.Guid.Data4[5], p.Guid.Data4[6], p.Guid.Data4[7]);
    p.BlockCount = blocks; p.BlockLength = 512;
    memcpy(p.ProductId, read_only ? "winluks RO" : "winluks RW", 10); memcpy(p.ProductRevisionLevel, "0301", 4);
    p.WriteProtected = read_only != 0; p.CacheSupported = 0; p.UnmapSupported = 0;
    p.MaxTransferLength = 1024 * 1024;
    error = SpdStorageUnitCreate(0, &p, &iface, &s->unit);
    if (error) { CloseHandle(s->publish_guard); HeapFree(GetProcessHeap(), 0, s); return error; }
    s->unit->UserContext = s;
    error = SpdStorageUnitStartDispatcher(s->unit, 1);
    if (error) {
        SpdStorageUnitShutdown(s->unit);
        SpdStorageUnitWaitDispatcher(s->unit);
        SpdStorageUnitDelete(s->unit);
        CloseHandle(s->publish_guard);
        HeapFree(GetProcessHeap(), 0, s); return error;
    }
    *out = s; return ERROR_SUCCESS;
}
DWORD wl_error(WL_SESSION *s) {
    DWORD error = 0;
    SpdStorageUnitGetDispatcherError(s->unit, &error); return error;
}

/* Never lock/dismount by drive letter alone. WinBtrfs forwards the storage query to
 * its backing disk, including for partitionless volumes without disk-extents IOCTL.
 * Match the exact random SCSI serial created for this session. Queries cannot write. */
static int belongs_to_session(WL_SESSION *s, HANDLE volume) {
    STORAGE_PROPERTY_QUERY query = {StorageDeviceProperty, PropertyStandardQuery, {0}};
    BYTE descriptor[4096];
    STORAGE_DEVICE_DESCRIPTOR *d = (void *)descriptor;
    DWORD n;
    int match = 0;
    memset(descriptor, 0, sizeof descriptor);
    if (DeviceIoControl(volume, IOCTL_STORAGE_QUERY_PROPERTY, &query, sizeof query,
        descriptor, sizeof descriptor, &n, 0) && n >= sizeof *d && n <= sizeof descriptor && d->SerialNumberOffset &&
        d->SerialNumberOffset < n && n - d->SerialNumberOffset > 36) {
        match = !_strnicmp((char *)descriptor + d->SerialNumberOffset, s->serial, 36) &&
            descriptor[d->SerialNumberOffset + 36] == 0;
    }
    return match;
}
static DWORD find_volume(WL_SESSION *s, HANDLE *out, wchar_t root[64], DWORD access) {
    wchar_t name[64], open_name[64];
    HANDLE search, volume;
    DWORD error = ERROR_NOT_READY;
    *out = INVALID_HANDLE_VALUE;
    search = FindFirstVolumeW(name, 64);
    if (search == INVALID_HANDLE_VALUE) return GetLastError();
    do {
        size_t len = wcslen(name);
        if (!len || name[len-1] != L'\\') continue;
        wcscpy_s(open_name, 64, name); open_name[len-1] = 0;
        volume = CreateFileW(open_name, access, FILE_SHARE_READ | FILE_SHARE_WRITE,
            0, OPEN_EXISTING, 0, 0);
        if (volume == INVALID_HANDLE_VALUE) continue;
        if (belongs_to_session(s, volume)) {
            if (*out != INVALID_HANDLE_VALUE) {
                CloseHandle(volume); CloseHandle(*out); *out = INVALID_HANDLE_VALUE;
                error = ERROR_DUP_NAME; break;
            }
            *out = volume; wcscpy_s(root, 64, name); error = ERROR_SUCCESS;
        } else CloseHandle(volume);
    } while (FindNextVolumeW(search, name, 64));
    FindVolumeClose(search);
    return error;
}
DWORD wl_volume_ready(WL_SESSION *s) {
    HANDLE volume;
    wchar_t root[64], filesystem[32];
    DWORD serial, max, flags, error = find_volume(s, &volume, root, 0);
    if (error) return error;
    CloseHandle(volume);
    if (!GetVolumeInformationW(root, 0, 0, &serial, &max, &flags, filesystem, 32)) return GetLastError();
    if (_wcsicmp(filesystem, L"BTRFS")) return ERROR_UNRECOGNIZED_VOLUME;
    if (!!(flags & FILE_READ_ONLY_VOLUME) != !!s->unit->StorageUnitParams.WriteProtected)
        return ERROR_WRITE_PROTECT;
    return ERROR_SUCCESS;
}
/* All mode/identity checks refer to the already opened volume handle. */
static DWORD volume_mode(HANDLE volume, int readonly) {
    wchar_t filesystem[32];
    DWORD serial, max, flags;
    if (!GetVolumeInformationByHandleW(volume, 0, 0, &serial, &max, &flags, filesystem, 32)) return GetLastError();
    if (_wcsicmp(filesystem, L"BTRFS")) return ERROR_UNRECOGNIZED_VOLUME;
    if (!!(flags & FILE_READ_ONLY_VOLUME) != !!readonly) return ERROR_WRITE_PROTECT;
    return ERROR_SUCCESS;
}
DWORD wl_lock_and_dismount(WL_SESSION *s, DWORD *phase) {
    HANDLE volume;
    wchar_t root[64];
    DWORD n, error;
    *phase = 0; /* lookup/mode; only phase 1 sharing failures are retryable */
    if (s->locked_volume) return ERROR_SUCCESS;
    error = find_volume(s, &volume, root, GENERIC_READ | GENERIC_WRITE);
    if (error) return error;
    error = volume_mode(volume, 0);
    if (error) { CloseHandle(volume); return error; }
    *phase = 1;
    if (!DeviceIoControl(volume, FSCTL_LOCK_VOLUME, 0, 0, 0, 0, &n, 0)) {
        error = GetLastError();
        /* WinBtrfs can switch to RO after a failed transaction commit. */
        if (volume_mode(volume, 0)) { *phase = 0; error = ERROR_WRITE_PROTECT; }
        CloseHandle(volume); return error;
    }
    *phase = 2;
    error = volume_mode(volume, 0);
    if (!error && !FlushFileBuffers(volume)) error = GetLastError();
    if (!error) {
        *phase = 3;
        if (!DeviceIoControl(volume, FSCTL_DISMOUNT_VOLUME, 0, 0, 0, 0, &n, 0)) error = GetLastError();
    }
    if (error) {
        DeviceIoControl(volume, FSCTL_UNLOCK_VOLUME, 0, 0, 0, 0, &n, 0);
        CloseHandle(volume); return error;
    }
    *phase = 4;
    s->locked_volume = volume;
    return ERROR_SUCCESS;
}
DWORD wl_close(WL_SESSION *s) {
    DWORD before = 0, after = 0;
    if (!s) return ERROR_INVALID_PARAMETER;
    SpdStorageUnitGetDispatcherError(s->unit, &before);
    SpdStorageUnitShutdown(s->unit);
    SpdStorageUnitWaitDispatcher(s->unit);
    SpdStorageUnitGetDispatcherError(s->unit, &after);
    SpdStorageUnitDelete(s->unit);
    if (s->locked_volume) CloseHandle(s->locked_volume);
    CloseHandle(s->publish_guard);
    SecureZeroMemory(s, sizeof *s);
    HeapFree(GetProcessHeap(), 0, s);
    /* Explicit shutdown cancels the pending transport request with OPERATION_ABORTED. */
    return before ? before : (after == ERROR_OPERATION_ABORTED ? ERROR_SUCCESS : after);
}
