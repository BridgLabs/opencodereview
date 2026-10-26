import { useId } from "react";
import { cx } from "./DesignSystem";

/** A compact single choice between a few options, such as a date range. Native radios give arrow-key moves. */
export function SegmentedControl<T extends string | number>({
  label,
  options,
  value,
  onChange,
}: {
  label: string;
  options: Array<{ value: T; label: string }>;
  value: T;
  onChange: (value: T) => void;
}) {
  const name = useId();
  return (
    <fieldset className="m-0 inline-flex h-8 min-w-0 items-center gap-0.5 rounded-lg border-0 bg-surface-muted p-0.5">
      <legend className="sr-only">{label}</legend>
      {options.map((option) => (
        <label
          key={option.value}
          className={cx(
            "inline-flex h-7 cursor-pointer items-center rounded-md px-2.5 text-caption font-medium tabular-nums transition-colors has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-brand",
            option.value === value ? "bg-surface-sunken text-fg shadow-raised" : "text-fg-muted hover:text-fg",
          )}
        >
          <input
            type="radio"
            name={name}
            className="sr-only"
            checked={option.value === value}
            onChange={() => onChange(option.value)}
          />
          {option.label}
        </label>
      ))}
    </fieldset>
  );
}
