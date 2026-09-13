/* SPDX-License-Identifier: GPL-3.0-or-later */
/* Disposable-VM connectivity spike. Never linked into the production CLI. */
#include <windows.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
typedef int (*READ_FN)(void *, uint64_t, uint32_t, void *);
typedef int (*WRITE_FN)(void *, uint64_t, uint32_t, const void *);
typedef int (*CTL_FN)(void *, uint32_t, uint64_t, uint32_t);
DWORD wl_create(void *, READ_FN, WRITE_FN, CTL_FN, uint64_t, uint32_t, void **);
DWORD wl_error(void *);
DWORD wl_close(void *);
typedef struct { HANDLE file; uint64_t bytes, reads, writes, unmaps; } TEST_DISK;
static HANDLE stop_event;
static BOOL WINAPI stop(DWORD kind) { (void)kind; SetEvent(stop_event); return TRUE; }
static int read_blocks(void *ctx, uint64_t lba, uint32_t count, void *buffer) {
    TEST_DISK *d=ctx;
    LARGE_INTEGER pos;
    DWORD bytes;
    if (count>2048 || lba>d->bytes/512 || count>d->bytes/512-lba) return 2;
    if (!count) return 0;
    pos.QuadPart=lba*512;
    d->reads++;
    if (!SetFilePointerEx(d->file,pos,0,FILE_BEGIN) || !ReadFile(d->file,buffer,count*512,&bytes,0) || bytes!=count*512) {
        SecureZeroMemory(buffer,count*512); return 4;
    }
    return 0;
}
static int control(void *ctx,uint32_t op,uint64_t lba,uint32_t count) {
    TEST_DISK *d=ctx;
    if(op==1) { d->writes++; return 1; }
    if(op==3) { d->unmaps++; return 1; }
    if(op==2) return lba<=d->bytes/512 && (!count || count<=d->bytes/512-lba) ? 0:2;
    return 2;
}
static int write_blocks(void *ctx,uint64_t lba,uint32_t count,const void *buffer) {
    (void)buffer; return control(ctx,1,lba,count);
}
int wmain(int argc,wchar_t **argv) {
    TEST_DISK d={0};
    LARGE_INTEGER size;
    void *session=0;
    DWORD rc, seconds=180;
    BY_HANDLE_FILE_INFORMATION info;
    if(argc<2||argc>3) { fputs("Usage: winluks-g0.exe <LOCAL-PLAINTEXT-FIXTURE> [seconds]\n",stderr); return 2; }
    if(argc==3) { seconds=wcstoul(argv[2],0,10); if(!seconds||seconds>3600) return 2; }
    d.file=CreateFileW(argv[1],GENERIC_READ,FILE_SHARE_READ,0,OPEN_EXISTING,FILE_FLAG_OPEN_REPARSE_POINT,0);
    if(d.file==INVALID_HANDLE_VALUE) return 3;
    if(GetFileType(d.file)!=FILE_TYPE_DISK||!GetFileInformationByHandle(d.file,&info)||
        info.dwFileAttributes&(FILE_ATTRIBUTE_DIRECTORY|FILE_ATTRIBUTE_REPARSE_POINT)||
        !GetFileSizeEx(d.file,&size)||size.QuadPart<=0||size.QuadPart%512) { CloseHandle(d.file);return 3; }
    d.bytes=size.QuadPart;
    stop_event=CreateEventW(0,TRUE,FALSE,0);
    if(!stop_event) { CloseHandle(d.file);return 3; }
    SetConsoleCtrlHandler(stop,TRUE);
    rc=wl_create(&d,read_blocks,write_blocks,control,d.bytes/512,1,&session);
    if(rc) { fprintf(stderr,"WINSPD_CREATE_ERROR code=%lu\n",rc);CloseHandle(d.file);CloseHandle(stop_event);return 4; }
    puts("PUBLISHED_RO G0_PLAINTEXT_FIXTURE");fflush(stdout);
    for(DWORD i=0;i<seconds;i++) { if(WaitForSingleObject(stop_event,1000)==WAIT_OBJECT_0||wl_error(session)) break; }
    rc=wl_error(session);{DWORD closed=wl_close(session);if(!rc)rc=closed;}
    printf("CLOSED reads=%llu write_callbacks=%llu unmap_callbacks=%llu dispatcher_error=%lu\n",d.reads,d.writes,d.unmaps,rc);
    CloseHandle(d.file);CloseHandle(stop_event);return rc?5:0;
}
