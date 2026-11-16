import { keymap } from "nsh";
import { IMPORT_LABELS, type ImportSource } from "../domain/import";
import { closeImportMenu, runImport } from "../store/importer";
import { importSources } from "../store/session";
import { MenuCard, MenuEntry, type IconName } from "../ui";

const icons: Record<ImportSource, IconName> = { photos: "image", files: "file", folder: "folder", pdf: "doc", paste: "clipboard" };

/** 导入菜单: 项目由宿主声明, 网页不会出现相册。 */
export function ImportMenu() {
  return (
    <MenuCard testID="import-menu" backdropTestID="import-backdrop" title="导入" onClose={closeImportMenu}>
      <Each of={importSources()} key={(source) => source}>
        {(source) => (
          <MenuEntry
            testID={`import-${source()}`}
            label={IMPORT_LABELS[source()]}
            icon={icons[source()]}
            hint={source() === "paste" ? keymap.label("mod+v") : undefined}
            onPress={() => runImport(source())}
          />
        )}
      </Each>
    </MenuCard>
  );
}
