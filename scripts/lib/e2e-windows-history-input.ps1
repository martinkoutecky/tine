param(
  [Parameter(Mandatory)][int]$ProcessId,
  [Parameter(Mandatory)][ValidateRange(1, 2)][int]$Button,
  [Parameter(Mandatory)][ValidateSet('mouse', 'native', 'native-first', 'child-only')][string]$Delivery
)
$ErrorActionPreference = 'Stop'

Add-Type @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;

public static class HistoryInput {
    [StructLayout(LayoutKind.Sequential)]
    private struct Point { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { public int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput {
        public int X, Y;
        public uint MouseData, Flags, Time;
        public UIntPtr ExtraInfo;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Input {
        public uint Type;
        public MouseInput Mouse;
    }

    private const uint WM_APPCOMMAND = 0x0319;
    private const uint FAPPCOMMAND_MOUSE = 0x8000;
    private const uint MOUSEEVENTF_XDOWN = 0x0080;
    private const uint MOUSEEVENTF_XUP = 0x0100;
    private const uint GA_ROOT = 2;

    [DllImport("user32.dll")] public static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern bool GetClientRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll")] private static extern bool ClientToScreen(IntPtr window, ref Point point);
    [DllImport("user32.dll")] private static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] private static extern IntPtr WindowFromPoint(Point point);
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll", ExactSpelling = true)] private static extern IntPtr SendMessageW(IntPtr window, uint message, IntPtr origin, IntPtr data);
    [DllImport("user32.dll", SetLastError = true)] private static extern uint SendInput(uint count, Input[] inputs, int size);

    public static void Press(IntPtr window, uint button, string delivery) {
        if (GetForegroundWindow() != window)
            throw new InvalidOperationException("Tine is not the foreground window");

        if (delivery == "native") {
            AppCommand(window, window, button);
            return;
        }

        Rect rect;
        if (!GetClientRect(window, out rect)) throw new InvalidOperationException("Tine's client area is unavailable");
        var point = new Point { X = (rect.Left + rect.Right) / 2, Y = (rect.Top + rect.Bottom) / 2 };
        if (!ClientToScreen(window, ref point) || !SetCursorPos(point.X, point.Y))
            throw new InvalidOperationException("Could not position the pointer in Tine");
        var child = WindowFromPoint(point);
        if (child == IntPtr.Zero || child == window || GetAncestor(child, GA_ROOT) != window)
            throw new InvalidOperationException("The pointer is not over Tine's WebView2 child window");

        if (delivery == "native-first" || delivery == "child-only") AppCommand(window, child, button);
        if (delivery == "child-only") return;

        // INPUT_MOUSE is zero. The mouse member is the largest member of INPUT's union.
        var input = new[] {
            new Input { Mouse = new MouseInput { MouseData = button, Flags = MOUSEEVENTF_XDOWN } },
            new Input { Mouse = new MouseInput { MouseData = button, Flags = MOUSEEVENTF_XUP } }
        };
        if (SendInput((uint)input.Length, input, Marshal.SizeOf(typeof(Input))) != input.Length)
            throw new Win32Exception(Marshal.GetLastWin32Error(), "SendInput did not inject both XBUTTON events");
    }

    private static void AppCommand(IntPtr window, IntPtr origin, uint button) {
        // The XBUTTON1/2 values are also the Back/Forward APPCOMMAND values.
        var data = new IntPtr(unchecked((int)((FAPPCOMMAND_MOUSE | button) << 16)));
        if (SendMessageW(window, WM_APPCOMMAND, origin, data) == IntPtr.Zero)
            throw new InvalidOperationException("Tine did not process WM_APPCOMMAND");
    }
}
'@

$window = (Get-Process -Id $ProcessId).MainWindowHandle
if ($window -eq [IntPtr]::Zero) { throw 'Tine has no visible main window' }
if ([HistoryInput]::GetForegroundWindow() -ne $window) {
  [void](New-Object -ComObject WScript.Shell).AppActivate($ProcessId)
}
[HistoryInput]::Press($window, $Button, $Delivery)
