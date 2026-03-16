import { useState } from "react";
import { Button } from "@/components/ui/button";
import { X } from "lucide-react";

interface CreateDatasetFormProps {
  onSubmit: (name: string, description: string, targetEpisodes: number | null) => void;
  onCancel: () => void;
}

export function CreateDatasetForm({ onSubmit, onCancel }: CreateDatasetFormProps) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [targetEpisodes, setTargetEpisodes] = useState("");

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;
    const target = targetEpisodes.trim() ? parseInt(targetEpisodes.trim(), 10) : null;
    onSubmit(name.trim(), description.trim(), target && target > 0 ? target : null);
  };

  return (
    <form onSubmit={handleSubmit} className="rounded-lg border border-border bg-card p-4 space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">New Dataset</h3>
        <button type="button" onClick={onCancel} className="text-muted-foreground hover:text-foreground">
          <X className="size-4" />
        </button>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            Name
          </label>
          <input
            type="text"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. grasp-mug-session"
            className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
            autoFocus
          />
        </div>

        <div className="space-y-1.5">
          <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
            Target Episodes (optional)
          </label>
          <input
            type="number"
            value={targetEpisodes}
            onChange={(e) => setTargetEpisodes(e.target.value)}
            placeholder="e.g. 50"
            min={1}
            className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
          />
        </div>
      </div>

      <div className="space-y-1.5">
        <label className="text-[11px] font-medium text-muted-foreground uppercase tracking-wider">
          Description (optional)
        </label>
        <input
          type="text"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="Brief description of the dataset"
          className="w-full h-8 px-3 rounded-md border border-input bg-background text-foreground placeholder:text-muted-foreground text-sm"
        />
      </div>

      <div className="flex gap-2 justify-end">
        <Button type="button" variant="outline" size="sm" onClick={onCancel}>
          Cancel
        </Button>
        <Button type="submit" size="sm" disabled={!name.trim()}>
          Create
        </Button>
      </div>
    </form>
  );
}
