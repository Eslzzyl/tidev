import { useEffect, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { X } from "lucide-react";
import { useTranslation } from "react-i18next";

import type { MessageAttachment } from "../../types/api";
import type { PendingImage } from "../../utils/imageAttachments";

function ImagePreviewOverlay({
  url,
  alt,
  onClose,
}: {
  url: string;
  alt: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();

  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };

    document.body.style.overflow = "hidden";
    document.addEventListener("keydown", handleKeyDown);
    return () => {
      document.body.style.overflow = previousOverflow;
      document.removeEventListener("keydown", handleKeyDown);
    };
  }, [onClose]);

  return createPortal(
    <div
      className="image-preview-overlay"
      role="dialog"
      aria-modal="true"
      aria-label={t("Expand image")}
      onClick={onClose}
    >
      <button
        type="button"
        className="image-preview-close"
        onClick={onClose}
        title={t("Close")}
        aria-label={t("Close")}
      >
        <X size={22} />
      </button>
      <img src={url} alt={alt} onClick={(event) => event.stopPropagation()} />
    </div>,
    document.body,
  );
}

export function ImageAttachmentStrip({
  images,
  onRemove,
  disabled = false,
}: {
  images: PendingImage[];
  onRemove: (id: string) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const [previewId, setPreviewId] = useState<string | null>(null);
  const previewImage = images.find((image) => image.id === previewId);
  if (images.length === 0) return null;

  return (
    <>
      <div className="composer-image-attachments" aria-label={t("Pasted images")}>
        {images.map((image) => (
          <div className="composer-image-attachment" key={image.id}>
            <button
              type="button"
              className="composer-image-attachment-preview"
              onClick={() => setPreviewId(image.id)}
              title={t("Expand image")}
              aria-label={t("Expand image")}
            >
              <img src={image.previewUrl} alt={image.filename} />
            </button>
            <button
              type="button"
              className="composer-image-attachment-remove"
              disabled={disabled}
              onClick={() => {
                setPreviewId((current) => (current === image.id ? null : current));
                onRemove(image.id);
              }}
              title={t("Remove image")}
              aria-label={t("Remove image")}
            >
              <X size={18} />
            </button>
          </div>
        ))}
      </div>
      {previewImage ? (
        <ImagePreviewOverlay
          url={previewImage.previewUrl}
          alt={previewImage.filename}
          onClose={() => setPreviewId(null)}
        />
      ) : null}
    </>
  );
}

function imageAttachments(attachments: MessageAttachment[]) {
  return attachments.filter(
    (attachment): attachment is Extract<MessageAttachment, { type: "image" }> =>
      attachment.type === "image" && attachment.data.length > 0,
  );
}

export function MessageImageGallery({ attachments }: { attachments: MessageAttachment[] }) {
  const { t } = useTranslation();
  const [previewIndex, setPreviewIndex] = useState<number | null>(null);
  const previews = useMemo(
    () =>
      imageAttachments(attachments).map((image) => ({
        image,
        url: URL.createObjectURL(new Blob([Uint8Array.from(image.data)], { type: image.mime })),
      })),
    [attachments],
  );

  useEffect(
    () => () => {
      for (const preview of previews) URL.revokeObjectURL(preview.url);
    },
    [previews],
  );

  const preview = previewIndex === null ? undefined : previews[previewIndex];
  if (previews.length === 0) return null;

  return (
    <>
      <div className="message-image-gallery">
        {previews.map(({ image, url }, index) => (
          <button
            type="button"
            className="message-image-link"
            key={`${image.filename}:${image.file_size}:${index}`}
            onClick={() => setPreviewIndex(index)}
            title={t("Expand image")}
            aria-label={t("Expand image")}
          >
            <img src={url} alt={image.filename} />
          </button>
        ))}
      </div>
      {preview ? (
        <ImagePreviewOverlay
          url={preview.url}
          alt={preview.image.filename}
          onClose={() => setPreviewIndex(null)}
        />
      ) : null}
    </>
  );
}
