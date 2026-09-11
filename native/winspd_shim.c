/* SPDX-License-Identifier: GPL-3.0-or-later */
/* ABI verified against WinSpd 55c53bc454afbbba38bd1692f52beb77ed59142f. */
#include <winspd/winspd.h>
#include <objbase.h>
#include <stdint.h>
#include <stddef.h>

typedef char assert_params_size[(sizeof(SPD_STORAGE_UNIT_PARAMS) == 128) ? 1 : -1];
typedef char assert_status_size[(sizeof(SPD_STORAGE_UNIT_STATUS) == 32) ? 1 : -1];
typedef char assert_interface_size[(sizeof(SPD_STORAGE_UNIT_INTERFACE) == 16 * sizeof(void *)) ? 1 : -1];

typedef int (*WL_READ)(void *, uint64_t, uint32_t, void *);
typedef int (*WL_CONTROL)(void *, uint32_t, uint64_t, uint32_t);
typedef struct {
    SPD_STORAGE_UNIT *unit;
    void *context;
    WL_READ read;
    WL_CONTROL control;
} WL_SESSION;

static void result(SPD_STORAGE_UNIT_STATUS *s, int code) {
    memset(s, 0, sizeof *s);
    if (!code) return;
    s->ScsiStatus = 2; /* CHECK CONDITION */
    switch (code) {
        case 1: s->SenseKey = 7; s->ASC = 0x27; break; /* DATA PROTECT */
        case 2: s->SenseKey = 5; s->ASC = 0x21; break; /* LBA OUT OF RANGE */
        case 3: s->SenseKey = 2; s->ASC = 0x04; break; /* NOT READY */
        default: s->SenseKey = 3; s->ASC = 0x11; break; /* UNRECOVERED READ */
    }
}
static BOOLEAN read_cb(SPD_STORAGE_UNIT *u, void *buffer, UINT64 lba, UINT32 count,
    BOOLEAN fua, SPD_STORAGE_UNIT_STATUS *status) {
    WL_SESSION *s = u->UserContext;
    (void)fua; /* RO device has no dirty cache. */
    if (count > 2048 || (count && !buffer)) { result(status, 2); return TRUE; }
    result(status, s->read(s->context, lba, count, buffer));
    /* TRUE means completed; it does not mean successful. */
    return TRUE;
}
static BOOLEAN write_cb(SPD_STORAGE_UNIT *u, void *buffer, UINT64 lba, UINT32 count,
    BOOLEAN fua, SPD_STORAGE_UNIT_STATUS *status) {
    WL_SESSION *s = u->UserContext;
    (void)buffer; (void)fua;
    s->control(s->context, 1, lba, count);
    result(status, 1); return TRUE;
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
    s->control(s->context, 3, 0, count);
    result(status, 1); return TRUE;
}
static const SPD_STORAGE_UNIT_INTERFACE iface = {read_cb, write_cb, flush_cb, unmap_cb, {0}};

DWORD wl_create(void *context, WL_READ read, WL_CONTROL control, uint64_t blocks, WL_SESSION **out) {
    SPD_STORAGE_UNIT_PARAMS p = {0};
    WL_SESSION *s;
    DWORD error;
    *out = 0;
    if (!blocks || !context || !read || !control) return ERROR_INVALID_PARAMETER;
    s = HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, sizeof *s);
    if (!s) return ERROR_NOT_ENOUGH_MEMORY;
    s->context = context; s->read = read; s->control = control;
    if (FAILED(CoCreateGuid(&p.Guid))) { HeapFree(GetProcessHeap(), 0, s); return ERROR_GEN_FAILURE; }
    p.BlockCount = blocks; p.BlockLength = 512;
    memcpy(p.ProductId, "winluks RO", 10); memcpy(p.ProductRevisionLevel, "0200", 4);
    p.WriteProtected = 1; p.CacheSupported = 0; p.UnmapSupported = 0;
    p.MaxTransferLength = 1024 * 1024;
    error = SpdStorageUnitCreate(0, &p, &iface, &s->unit);
    if (error) { HeapFree(GetProcessHeap(), 0, s); return error; }
    s->unit->UserContext = s;
    error = SpdStorageUnitStartDispatcher(s->unit, 1);
    if (error) {
        SpdStorageUnitShutdown(s->unit);
        SpdStorageUnitWaitDispatcher(s->unit);
        SpdStorageUnitDelete(s->unit);
        HeapFree(GetProcessHeap(), 0, s); return error;
    }
    *out = s; return ERROR_SUCCESS;
}
DWORD wl_error(WL_SESSION *s) {
    DWORD error = 0;
    SpdStorageUnitGetDispatcherError(s->unit, &error); return error;
}
void wl_close(WL_SESSION *s) {
    if (!s) return;
    SpdStorageUnitShutdown(s->unit);
    SpdStorageUnitWaitDispatcher(s->unit);
    SpdStorageUnitDelete(s->unit);
    SecureZeroMemory(s, sizeof *s);
    HeapFree(GetProcessHeap(), 0, s);
}
