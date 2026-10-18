import { type ReactElement } from "react";
type Values = Record<string, string | number | Date>;
export declare function getGlotTranslations(namespace?: string): Promise<(key: string, values?: Values) => ReactElement>;
export {};
