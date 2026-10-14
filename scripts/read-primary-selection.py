"""Independent X11 client reads PRIMARY exactly as a middle-click paste target."""
import ctypes as c
import ctypes.util
import json
import time

x = c.CDLL(ctypes.util.find_library("X11"))
x.XOpenDisplay.restype = c.c_void_p
x.XDefaultRootWindow.argtypes = [c.c_void_p]
x.XDefaultRootWindow.restype = c.c_ulong
x.XCreateSimpleWindow.argtypes = [c.c_void_p, c.c_ulong, c.c_int, c.c_int, c.c_uint, c.c_uint, c.c_uint, c.c_ulong, c.c_ulong]
x.XCreateSimpleWindow.restype = c.c_ulong
x.XInternAtom.argtypes = [c.c_void_p, c.c_char_p, c.c_int]
x.XInternAtom.restype = c.c_ulong
x.XConvertSelection.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_ulong, c.c_ulong, c.c_ulong]
x.XFlush.argtypes = [c.c_void_p]
x.XPending.argtypes = [c.c_void_p]
x.XNextEvent.argtypes = [c.c_void_p, c.c_void_p]
x.XGetWindowProperty.argtypes = [c.c_void_p, c.c_ulong, c.c_ulong, c.c_long, c.c_long, c.c_int, c.c_ulong, c.POINTER(c.c_ulong), c.POINTER(c.c_int), c.POINTER(c.c_ulong), c.POINTER(c.c_ulong), c.POINTER(c.c_void_p)]
x.XFree.argtypes = [c.c_void_p]
x.XDestroyWindow.argtypes = [c.c_void_p, c.c_ulong]
x.XCloseDisplay.argtypes = [c.c_void_p]

class Selection(c.Structure):
    _fields_ = [("type", c.c_int), ("serial", c.c_ulong), ("send", c.c_int), ("display", c.c_void_p), ("requestor", c.c_ulong), ("selection", c.c_ulong), ("target", c.c_ulong), ("property", c.c_ulong), ("time", c.c_ulong)]

display = x.XOpenDisplay(None)
if not display:
    raise RuntimeError("X11 display unavailable")
window = x.XCreateSimpleWindow(display, x.XDefaultRootWindow(display), 0, 0, 1, 1, 0, 0, 0)

def read(target):
    atom = x.XInternAtom(display, target.encode(), 0)
    prop = x.XInternAtom(display, b"TINE_SELECTION_PROBE", 0)
    x.XConvertSelection(display, 1, atom, prop, window, 0)  # XA_PRIMARY, CurrentTime
    x.XFlush(display)
    deadline = time.monotonic() + 5
    event = (c.c_long * 24)()
    while time.monotonic() < deadline:
        if not x.XPending(display):
            time.sleep(0.01)
            continue
        x.XNextEvent(display, c.byref(event))
        selection = c.cast(c.byref(event), c.POINTER(Selection)).contents
        if selection.type != 31:
            continue
        if not selection.property:
            return None
        actual, bits, items, remaining, data = c.c_ulong(), c.c_int(), c.c_ulong(), c.c_ulong(), c.c_void_p()
        x.XGetWindowProperty(display, window, prop, 0, 262144, 1, 0, c.byref(actual), c.byref(bits), c.byref(items), c.byref(remaining), c.byref(data))
        try:
            if bits.value != 8 or remaining.value:
                raise RuntimeError("Selection was not bounded byte text")
            return c.string_at(data, items.value).decode("utf-8") if data else ""
        finally:
            if data:
                x.XFree(data)
    raise RuntimeError("PRIMARY owner did not answer")

try:
    print(json.dumps({"text": read("UTF8_STRING"), "html": read("text/html")}))
finally:
    x.XDestroyWindow(display, window)
    x.XCloseDisplay(display)
