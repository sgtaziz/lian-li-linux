import { defineComponent, h, type VNodeChild } from "vue";
import { NSelect, type SelectOption, type SelectGroupOption } from "naive-ui";

type AnyOption = SelectOption | SelectGroupOption;
type RenderLabel = (option: SelectOption, selected: boolean) => VNodeChild;

function text(option: AnyOption | undefined): string | undefined {
  return typeof option?.label === "string" ? option.label : undefined;
}

function find(options: AnyOption[] | undefined, value: unknown): AnyOption | undefined {
  for (const option of options ?? []) {
    if (option.type === "group") {
      const nested = find((option as SelectGroupOption).children as AnyOption[], value);
      if (nested) return nested;
    } else if ((option as SelectOption).value === value) {
      return option;
    }
  }
  return undefined;
}

// Long labels are clipped in the field and menu, so both show the full text on hover.
export default defineComponent({
  name: "TitledSelect",
  inheritAttrs: false,
  setup(_, { attrs, slots }) {
    return () => {
      const { renderLabel: camel, "render-label": kebab, ...rest } = attrs;
      const custom = (camel ?? kebab) as RenderLabel | undefined;
      const renderLabel: RenderLabel = (option, selected) =>
        h("span", { title: text(option) }, [custom ? custom(option, selected) : (option.label as VNodeChild)]);
      const value = rest.value;
      const selected = Array.isArray(value) ? undefined : text(find(rest.options as AnyOption[] | undefined, value));
      return h(NSelect, { title: selected, ...rest, renderLabel }, slots);
    };
  },
});
