import { effect } from "nsh";
import { SORT_CHOICES, sortChoiceId, sortChoiceLabel } from "../domain/arrange";
import { displayIndexOf, displayOrder, itemOf, moveItemToBack, moveItemToFront, sortItems } from "../store/jobs";
import { closeMenu } from "../store/menu";
import { MenuCard, MenuEntry } from "../ui";
import { titleOf } from "./JobRow";

/**
 * 排序菜单: 选一项就把现有的所有条目排一次, 然后收起。只是一次性的整理, 之后加入的条目追加在末尾,
 * 不会自动保持排序; 排序只改界面上的顺序, 不影响提交给 Daemon 的先后。
 */
export function SortMenu() {
  return (
    <MenuCard testID="sort-menu" backdropTestID="sort-backdrop" title="排序" onClose={closeMenu}>
      <Each of={SORT_CHOICES} key={sortChoiceId}>
        {(choice) => (
          <MenuEntry
            testID={`sort-${sortChoiceId(choice())}`}
            label={sortChoiceLabel(choice())}
            icon="sort"
            onPress={() => {
              closeMenu();
              sortItems(choice());
            }}
          />
        )}
      </Each>
    </MenuCard>
  );
}

/** 条目菜单: 把这一项移到最前或最后。已经在那里的一项置灰。 */
export function ItemMenu(props: { itemKey: string }) {
  // 菜单开着时这一项被移除了 (轮询发现 Job 没了、用户按了 Backspace), 菜单跟着收起。
  effect(() => {
    displayOrder();
    if (itemOf(props.itemKey) === undefined) closeMenu();
  });
  const index = () => {
    displayOrder();
    return displayIndexOf(props.itemKey);
  };
  const move = (run: (key: string) => void) => () => {
    // 先取下 key: 菜单一收起, 条目菜单的 props 就跟着没了。
    const key = props.itemKey;
    closeMenu();
    run(key);
  };
  return (
    <If when={itemOf(props.itemKey)}>
      {(item) => (
        <MenuCard testID="item-menu" backdropTestID="item-menu-backdrop" title={titleOf(item())} onClose={closeMenu}>
          <MenuEntry testID="move-to-front" label="移到最前" icon="to-top" disabled={index() <= 0} onPress={move(moveItemToFront)} />
          <MenuEntry
            testID="move-to-back"
            label="移到最后"
            icon="to-bottom"
            disabled={index() < 0 || index() >= displayOrder().length - 1}
            onPress={move(moveItemToBack)}
          />
        </MenuCard>
      )}
    </If>
  );
}