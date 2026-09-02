namespace Lattice.Control.Desktop;

internal enum WindowResizeHit
{
    Client = 1,
    Left = 10,
    Right = 11,
    Top = 12,
    TopLeft = 13,
    TopRight = 14,
    Bottom = 15,
    BottomLeft = 16,
    BottomRight = 17,
}

internal readonly record struct WindowResizeInsets(
    double Left,
    double Top,
    double Right,
    double Bottom);

internal static class WindowResizeHitTestPolicy
{
    public const int WmNcHitTest = 0x0084;

    public static WindowResizeHit EvaluatePhysical(
        int screenX,
        int screenY,
        int windowLeft,
        int windowTop,
        int windowRight,
        int windowBottom,
        double dpiScaleX,
        double dpiScaleY,
        WindowResizeInsets insets,
        bool isMaximized)
    {
        if (
            isMaximized
            || !double.IsFinite(dpiScaleX)
            || !double.IsFinite(dpiScaleY)
            || dpiScaleX <= 0
            || dpiScaleY <= 0
            || windowRight <= windowLeft
            || windowBottom <= windowTop)
        {
            return WindowResizeHit.Client;
        }

        double width = (windowRight - windowLeft) / dpiScaleX;
        double height = (windowBottom - windowTop) / dpiScaleY;
        double x = (screenX - windowLeft) / dpiScaleX;
        double y = (screenY - windowTop) / dpiScaleY;
        return EvaluateDip(x, y, width, height, insets);
    }

    private static WindowResizeHit EvaluateDip(
        double x,
        double y,
        double width,
        double height,
        WindowResizeInsets insets)
    {
        if (
            !double.IsFinite(x)
            || !double.IsFinite(y)
            || !double.IsFinite(width)
            || !double.IsFinite(height)
            || width <= 0
            || height <= 0
            || x < 0
            || y < 0
            || x >= width
            || y >= height)
        {
            return WindowResizeHit.Client;
        }

        double leftInset = BoundedInset(insets.Left, width);
        double rightInset = BoundedInset(insets.Right, width);
        double topInset = BoundedInset(insets.Top, height);
        double bottomInset = BoundedInset(insets.Bottom, height);
        bool left = x < leftInset;
        bool right = x >= width - rightInset;
        bool top = y < topInset;
        bool bottom = y >= height - bottomInset;

        if (top && left) return WindowResizeHit.TopLeft;
        if (top && right) return WindowResizeHit.TopRight;
        if (bottom && left) return WindowResizeHit.BottomLeft;
        if (bottom && right) return WindowResizeHit.BottomRight;
        if (left) return WindowResizeHit.Left;
        if (right) return WindowResizeHit.Right;
        if (top) return WindowResizeHit.Top;
        if (bottom) return WindowResizeHit.Bottom;
        return WindowResizeHit.Client;
    }

    private static double BoundedInset(double value, double extent)
    {
        if (!double.IsFinite(value) || value <= 0) return 0;
        return Math.Min(value, extent / 2);
    }
}
