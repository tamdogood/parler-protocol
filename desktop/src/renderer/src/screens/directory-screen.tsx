import type { HubTarget } from "@shared/types";
import { Directory } from "@/components/directory";

export function DirectoryScreen({ base, target }: { base: string | null; target: HubTarget }) {
  return (
    <div className="mx-auto max-w-[1120px] px-8 py-8">
      {base ? (
        <Directory key={target} base={base} canViewHub />
      ) : (
        <p className="text-[13px] text-steel">Resolving hub…</p>
      )}
    </div>
  );
}
