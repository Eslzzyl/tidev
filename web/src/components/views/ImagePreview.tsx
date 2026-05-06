import { useState, useEffect } from "react";
import { Loader2, AlertCircle } from "lucide-react";
import { api } from "../../api/client";

interface ImagePreviewProps {
  path: string;
}

export function ImagePreview({ path }: ImagePreviewProps) {
  const [src, setSrc] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    api
      .readFileBase64(path)
      .then((res) => {
        if (!cancelled) {
          setSrc(`data:${res.mime};base64,${res.data}`);
          setLoading(false);
        }
      })
      .catch((err) => {
        if (!cancelled) {
          setError(err instanceof Error ? err.message : "Failed to load image");
          setLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [path]);

  if (loading) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-5 w-5 animate-spin text-neutral-400" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full flex-col items-center justify-center gap-2 p-8">
        <AlertCircle className="h-6 w-6 text-red-500" />
        <p className="text-xs text-red-500">{error}</p>
      </div>
    );
  }

  return (
    <div className="flex h-full items-center justify-center overflow-auto p-4">
      {src && (
        <img
          src={src}
          alt={path}
          className="max-h-full max-w-full rounded object-contain shadow-lg"
        />
      )}
    </div>
  );
}
