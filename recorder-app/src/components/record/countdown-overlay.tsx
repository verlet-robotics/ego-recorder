export function CountdownOverlay({ count }: { count: number }) {
  return (
    <div className="absolute inset-0 z-50 flex items-center justify-center bg-background/80 backdrop-blur-sm">
      <div className="text-center">
        {count > 0 ? (
          <span className="text-8xl font-serif font-bold text-foreground animate-in zoom-in-50 duration-200">
            {count}
          </span>
        ) : (
          <span className="text-6xl font-serif font-bold text-highlight animate-in zoom-in-50 duration-200">
            GO
          </span>
        )}
      </div>
    </div>
  );
}
