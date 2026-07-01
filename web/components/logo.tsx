/**
 * Parler brand mark — an electric-blue orbit with a lone satellite dot circling a violet nucleus.
 * Rendered transparent (no box) so it reads directly on the black canvas in the nav and footer;
 * the boxed app-icon version lives in `app/icon.svg` / `app/apple-icon.png`.
 */
export function Logo({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 32 32"
      fill="none"
      xmlns="http://www.w3.org/2000/svg"
      className={className}
      aria-hidden="true"
    >
      <circle cx="16" cy="16" r="8.2" stroke="#3b9eff" strokeWidth="1.8" />
      <circle cx="21.7" cy="10.2" r="1.9" fill="#3b9eff" />
      <circle cx="16" cy="16" r="2.6" fill="#9281f7" />
    </svg>
  );
}
