export interface paths {
    "/events": {
        parameters: {
            query?: never;
            header?: never;
            path?: never;
            cookie?: never;
        };
        get: {
            parameters: {
                query?: {
                    since?: Date;
                };
                header?: never;
                path?: never;
                cookie?: never;
            };
            requestBody?: never;
            responses: {
                /** @description ok */
                200: {
                    headers: {
                        [name: string]: unknown;
                    };
                    content: {
                        "application/json": ((Date | null) | undefined)[];
                    };
                };
            };
        };
        put?: never;
        post?: never;
        delete?: never;
        options?: never;
        head?: never;
        patch?: never;
        trace?: never;
    };
}
export type webhooks = Record<string, never>;
export interface components {
    schemas: {
        Event: {
            // #/components/schemas/Event/id
            /** @description Identifier */
            id: string;
            /**
             * Format: date-time
             * @description When
             */
            at: Date;
            /** Format: date */
            maybe?: Date | null;
            ref?: components["schemas"]["Stamp"];
            meta?: {
                // #/components/schemas/Event/meta/id
                id?: number;
                inner?: {
                    /**
                     * Format: json
                     * @description Raw
                     *     payload
                     */
                    payload?: {
                        /**
                        
                         * two
                         */
                        a: string;
                        b?: number;
                    };
                    tags?: ((Date | null) | undefined)[];
                };
            };
            $defs: {
                // #/components/schemas/Event/$defs/id
                /** Def id */
                id: string;
            };
        };
        /** Format: date-time */
        Stamp: Date;
        Wrapped: {
            x?: number;
        } | Wrapped;
    };
    responses: {
        /** @description empty */
        Empty: {
            headers: {
                [name: string]: unknown;
            };
            content?: never;
        };
    };
    parameters: never;
    requestBodies: never;
    headers: never;
    pathItems: never;
}
export type $defs = Record<string, never>;
export type operations = Record<string, never>;
