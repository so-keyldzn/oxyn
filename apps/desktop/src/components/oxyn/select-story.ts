import { expect, userEvent, waitFor, within } from "storybook/test"

/** A portalled Select closes before the next interaction or axe audit. */
export async function chooseSelectOption(
  trigger: HTMLElement,
  name: string | RegExp
) {
  await userEvent.click(trigger)
  const list = await within(document.body).findByRole("listbox")
  const popup = list.closest('[data-slot="select-content"]')
  await userEvent.click(await within(list).findByRole("option", { name }))
  // Base UI keeps closed options mounted for typeahead. Its positioner must
  // be hidden; a containing Dialog may still own other focus guards.
  await waitFor(() => expect(popup).not.toBeVisible())
  await expect(trigger).toHaveAttribute("aria-expanded", "false")
}
