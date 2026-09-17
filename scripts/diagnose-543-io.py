"""Disposable synthetic OS file-call probe; no SQLite or user data."""
import ctypes,json,os,platform,shutil,tempfile,time
from pathlib import Path
SIZE=1700000000//4096*4096
OPS=200000
roots=[Path(tempfile.gettempdir()),Path.cwd()]
for root in roots:
    folder=Path(tempfile.mkdtemp(prefix='543-io-',dir=root));path=folder/'probe.bin'
    try:
        start=time.perf_counter()
        with path.open('wb',buffering=0) as f:
            chunk=b'x'*1048576
            remaining=SIZE
            while remaining:
                part=chunk[:min(len(chunk),remaining)];f.write(part);remaining-=len(part)
            os.fsync(f.fileno())
        init=time.perf_counter()-start
        fd=os.open(path,os.O_RDWR)
        data=b'y'*4096
        if os.name=='nt':
            import msvcrt
            from ctypes import wintypes as w
            class OVERLAPPED(ctypes.Structure):
                _fields_=[('Internal',ctypes.c_size_t),('InternalHigh',ctypes.c_size_t),('Offset',w.DWORD),('OffsetHigh',w.DWORD),('hEvent',w.HANDLE)]
            k=ctypes.WinDLL('kernel32',use_last_error=True);handle=w.HANDLE(msvcrt.get_osfhandle(fd));buf=ctypes.create_string_buffer(data);got=w.DWORD()
            for name in ['ReadFile','WriteFile']:
                fn=getattr(k,name);fn.argtypes=[w.HANDLE,ctypes.c_void_p,w.DWORD,ctypes.POINTER(w.DWORD),ctypes.POINTER(OVERLAPPED)];fn.restype=w.BOOL
            def op(offset,write):
                ov=OVERLAPPED();ov.Offset=offset;ov.OffsetHigh=offset>>32
                if not (k.WriteFile if write else k.ReadFile)(handle,buf,4096,ctypes.byref(got),ctypes.byref(ov)):raise ctypes.WinError(ctypes.get_last_error())
                assert got.value==4096
        else:
            def op(offset,write):
                if write:assert os.pwrite(fd,data,offset)==4096
                else:assert len(os.pread(fd,4096,offset))==4096
        rows=[]
        for write in [False,True,False]:
            seed=543;start=time.perf_counter()
            for i in range(OPS):
                seed=(1664525*seed+1013904223)&0xffffffff
                op((seed%(SIZE//4096))*4096,write)
            elapsed=time.perf_counter()-start
            sync=time.perf_counter();os.fsync(fd);sync=time.perf_counter()-sync
            rows.append({'write':write,'calls':OPS,'bytes':OPS*4096,'elapsed':elapsed,'sync':sync})
        os.close(fd)
        print(json.dumps({'os':platform.platform(),'root':str(root),'file_bytes':SIZE,'initial_write_sync_seconds':init,'operations':rows}),flush=True)
    finally:shutil.rmtree(folder)
