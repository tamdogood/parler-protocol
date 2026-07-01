/**
 * The Parler orbit mark, built from plain divs so it renders reliably inside next/og (Satori):
 * an electric-blue ring with a lone satellite dot, around a violet nucleus. Used on the branded
 * social cards where the black app-icon box isn't needed (the card is already black).
 */
export function OgMark({ size = 44 }: { size?: number }) {
  const border = Math.round(size * 0.09);
  const nucleus = Math.round(size * 0.3);
  const dot = Math.round(size * 0.2);
  return (
    <div
      style={{
        position: "relative",
        width: size,
        height: size,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
        border: `${border}px solid #3b9eff`,
        borderRadius: 999,
        boxSizing: "border-box",
      }}
    >
      <div
        style={{ width: nucleus, height: nucleus, borderRadius: 999, background: "#9281f7" }}
      />
      <div
        style={{
          position: "absolute",
          top: Math.round(size * 0.06),
          left: Math.round(size * 0.72),
          width: dot,
          height: dot,
          borderRadius: 999,
          background: "#3b9eff",
        }}
      />
    </div>
  );
}
