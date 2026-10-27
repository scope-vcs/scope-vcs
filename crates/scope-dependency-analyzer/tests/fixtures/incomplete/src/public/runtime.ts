declare const selectedPath: string;
export const load = () => import(selectedPath);
export const loadAgain = () => require(selectedPath);
export const loadOptional = () => require?.("../private/value");
