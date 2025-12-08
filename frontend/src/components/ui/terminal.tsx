import React, { useEffect, useRef } from "react";
import { cn } from "../../lib/utils";
import { Loader2 } from "lucide-react";

interface TerminalProps extends React.HTMLAttributes<HTMLDivElement> {
  lines: string[];
  isLoading?: boolean;
  hasMore?: boolean;
  onScrollTop?: () => void;
  autoScroll?: boolean;
}

export const Terminal = React.forwardRef<HTMLDivElement, TerminalProps>(
  ({ className, lines, isLoading = false, hasMore = false, onScrollTop, autoScroll = true, ...props }, ref) => {
    const bottomRef = useRef<HTMLDivElement>(null);
    const containerRef = useRef<HTMLDivElement>(null);
    const prevScrollHeightRef = useRef<number>(0);

    useEffect(() => {
      if (autoScroll) {
        bottomRef.current?.scrollIntoView({ behavior: "smooth" });
      }
    }, [lines, autoScroll]);

    // Preserve scroll position when loading earlier content
    useEffect(() => {
      if (!containerRef.current) return;

      const container = containerRef.current;
      const prevScrollHeight = prevScrollHeightRef.current;

      if (prevScrollHeight > 0 && container.scrollHeight > prevScrollHeight) {
        const scrollDiff = container.scrollHeight - prevScrollHeight;
        container.scrollTop += scrollDiff;
      }

      prevScrollHeightRef.current = container.scrollHeight;
    }, [lines]);

    const handleScroll = (e: React.UIEvent<HTMLDivElement>) => {
      const container = e.currentTarget;

      // Check if scrolled to top (within 50px threshold)
      if (container.scrollTop < 50 && hasMore && !isLoading && onScrollTop) {
        onScrollTop();
      }
    };

    return (
      <div
        ref={(node) => {
          containerRef.current = node;
          if (typeof ref === 'function') {
            ref(node);
          } else if (ref) {
            ref.current = node;
          }
        }}
        className={cn(
          "bg-black text-green-400 font-mono p-4 rounded-lg overflow-auto max-h-[600px] text-sm",
          className
        )}
        onScroll={handleScroll}
        {...props}
      >
        {isLoading && hasMore && (
          <div className="flex items-center gap-2 text-yellow-400 mb-2">
            <Loader2 className="h-4 w-4 animate-spin" />
            <span>Loading earlier logs...</span>
          </div>
        )}
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