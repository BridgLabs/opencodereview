import mascotUrl from "../../assets/mascot.svg";

const pixels = { sm: 36, md: 64, lg: 128 } as const;

export function BrandMark({ size = "md" }: { size?: keyof typeof pixels }) {
  return (
    <div className={`brand-mark ${size}`} aria-hidden="true">
      <img src={mascotUrl} width={pixels[size]} height={pixels[size]} alt="" />
    </div>
  );
}
