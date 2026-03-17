import { useEffect, useRef } from "react";
import { createPortal } from "react-dom";
import { Button } from "./button";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description?: string;
  children?: React.ReactNode;
  confirmLabel?: string;
  cancelLabel?: string;
  destructive?: boolean;
  loading?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({
  open,
  title,
  description,
  children,
  confirmLabel = "Confirm",
  cancelLabel = "Cancel",
  destructive = false,
  loading = false,
  onConfirm,
  onCancel,
}: ConfirmDialogProps) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (open) cancelRef.current?.focus();
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onCancel]);

  if (!open) return null;

  return createPortal(
    <div className="fixed inset-0 z-50 flex items-center justify-center">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/60 animate-in fade-in-0"
        onClick={onCancel}
      />
      {/* Dialog */}
      <div className="relative z-10 w-full max-w-md rounded-lg border border-border bg-card p-6 shadow-lg animate-in fade-in-0 zoom-in-95">
        <h2 className="text-base font-semibold">{title}</h2>
        {description && (
          <p className="mt-1.5 text-sm text-muted-foreground">{description}</p>
        )}
        {children && <div className="mt-3">{children}</div>}
        <div className="mt-5 flex justify-end gap-2">
          <Button
            ref={cancelRef}
            variant="outline"
            size="sm"
            onClick={onCancel}
            disabled={loading}
          >
            {cancelLabel}
          </Button>
          <Button
            variant={destructive ? "destructive" : "default"}
            size="sm"
            onClick={onConfirm}
            disabled={loading}
          >
            {loading ? "Clearing..." : confirmLabel}
          </Button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
