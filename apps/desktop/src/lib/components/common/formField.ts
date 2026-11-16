/**
 * Attributes a FormRow hands to its control, to spread on it (`<Input {...field} />`):
 * the id its `<label for>` points at and the id of the help text that describes it.
 */
export interface FormFieldAttrs {
	id: string;
	'aria-describedby': string | undefined;
}

/**
 * Wires a FormRow's label and help to its control. `uid` is unique per row
 * (`$props.id()`); an explicit `controlId` wins over the generated one, so
 * existing controls keep their ids.
 */
export function formFieldIds(uid: string, controlId: string | undefined, hasHelp: boolean): { field: FormFieldAttrs; helpId: string | undefined } {
	const helpId = hasHelp ? `${uid}-help` : undefined;
	return { field: { id: controlId || `${uid}-control`, 'aria-describedby': helpId }, helpId };
}
