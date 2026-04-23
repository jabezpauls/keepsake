import { useEffect, useState } from "react";
import { api, bytesToBlobUrl } from "../../ipc";

interface Props {
    personId: number;
    size: number;
    alt?: string;
    className?: string;
}

// Fetches a face-cropped thumbnail for a person. The backend picks the
// highest-quality face for the person, decrypts its bbox, crops the source
// thumb1024 with padding and resizes to a square WebP — i.e. an
// Apple-Photos-style circular avatar once the page CSS adds border-radius.
export default function PersonFaceThumb({ personId, size, alt, className }: Props) {
    const [url, setUrl] = useState<string | null>(null);
    const [failed, setFailed] = useState(false);

    useEffect(() => {
        let cancelled = false;
        let currentUrl: string | null = null;
        setFailed(false);
        void (async () => {
            try {
                const bytes = await api.personFaceThumbnail(personId, size);
                if (cancelled) return;
                currentUrl = bytesToBlobUrl(bytes, "image/webp");
                setUrl(currentUrl);
            } catch {
                if (!cancelled) setFailed(true);
            }
        })();
        return () => {
            cancelled = true;
            if (currentUrl) URL.revokeObjectURL(currentUrl);
        };
    }, [personId, size]);

    if (failed) {
        return <div className={`thumb person-cover-empty ${className ?? ""}`} />;
    }
    if (!url) {
        return <div className={`thumb thumb-loading ${className ?? ""}`} />;
    }
    return (
        <img
            className={`thumb ${className ?? ""}`}
            src={url}
            alt={alt ?? ""}
            loading="lazy"
        />
    );
}
