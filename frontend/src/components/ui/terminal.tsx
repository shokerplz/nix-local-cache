import React, { useEffect, useRef } from "react";
import { cn } from "../../lib/utils";

interface TerminalProps extends React.HTMLAttributes<HTMLDivElement> {
  lines: string[];
}

export const Terminal = React.forwardRef<HTMLDivElement, TerminalProps>(
  ({ className, lines, ...props }, ref) => {
    const bottomRef = useRef<HTMLDivElement>(null);

    useEffect(() => {
      bottomRef.current?.scrollIntoView({ behavior: "smooth" });
    }, [lines]);

    return (
      <div
        ref={ref}
        className={cn(
          "bg-black text-green-400 font-mono p-4 rounded-lg overflow-auto max-h-[600px] text-sm",
          className
        )}
        {...props}
      >
        {lines.map((line, i) => (
          <div key={i} className="whitespace-pre-wrap break-all">
            {line}
          </div>
        ))}
        <div ref={bottomRef} />
      </div>
    );
  }
);
Terminal.displayName = "Terminal";