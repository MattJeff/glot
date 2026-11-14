// Landing icons generated with Higgsfield (gpt_image_2_6) on black, 128 px WebP in /public/landing/icons.
// .glow-img drops the black (screen in dark mode, inverted multiply in light mode).
export function Art({ n, className = "size-10" }: { n: string; className?: string }) {
  // eslint-disable-next-line @next/next/no-img-element
  return <img src={`/landing/icons/${n}.webp`} alt="" aria-hidden width={128} height={128} loading="lazy" decoding="async" className={`glow-img ${className}`} />;
}
