import { forwardRef } from "react";
import * as SliderPrimitive from "@radix-ui/react-slider";

import { cx } from "./utils";

export type SliderProps = React.ComponentPropsWithoutRef<typeof SliderPrimitive.Root>;

export const Slider = forwardRef<React.ElementRef<typeof SliderPrimitive.Root>, SliderProps>(
  function Slider({ className, value, defaultValue, ...props }, ref) {
    const thumbCount = value?.length ?? defaultValue?.length ?? 1;

    return (
      <SliderPrimitive.Root
        ref={ref}
        className={cx("ui-slider", className)}
        value={value}
        defaultValue={defaultValue}
        {...props}
      >
        <SliderPrimitive.Track className="ui-slider-track">
          <SliderPrimitive.Range className="ui-slider-range" />
        </SliderPrimitive.Track>
        {Array.from({ length: thumbCount }, (_, index) => (
          <SliderPrimitive.Thumb
            key={index}
            className="ui-slider-thumb"
            aria-label={props["aria-label"] ?? `Value ${index + 1}`}
          />
        ))}
      </SliderPrimitive.Root>
    );
  },
);
