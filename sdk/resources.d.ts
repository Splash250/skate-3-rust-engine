/** Resource API 1. Editor declarations; no Node.js, DOM, fetch or native modules. */
type ResourceJson = null | boolean | number | string | ResourceJson[] | { [key: string]: ResourceJson };
type ResourceScope = {kind: "resource"} | {kind: "instance"|"player"|"entity", id: string};
type ResourceHandler = (payload: ResourceJson, sender: string) => void;
interface ResourceApi {
  readonly id: string;
  readonly version: string;
  readonly generation: string;
  readonly side: 'client' | 'server';
  readonly grants: Readonly<Record<string, boolean>>;
  on(name: string, callback: ResourceHandler): void;
  onNet(name: string, callback: ResourceHandler): void;
  on_net(name: string, callback: ResourceHandler): void;
  off(name: string): void;
  emit(name: string, payload?: ResourceJson): void;
  send(name: string, payload?: ResourceJson, recipient?: string | null, scope?: ResourceScope | null): void;
  export(name: string, callback: (payload: ResourceJson) => ResourceJson): void;
  call(dependency: string, name: string, payload?: ResourceJson): ResourceJson;
  command(name: string, permission: string, callback: (args: string[], actor: string) => void): void;
  players(): ResourceJson;
  lifecycle(handlers: Partial<Record<'on_load'|'on_unload'|'on_update'|'on_fixed_update'|'on_ui_update'|'on_event'|'on_settings', (payload: any) => void>>): void;
  entities: {command(command: ResourceJson): void; all(): ResourceJson};
  voice: {submit(operation: ResourceVoiceServer | ResourceVoiceClient): void};
  world: {command(operation: ResourceJson): void};
  competition: {submit(operation: ResourceJson): void};
  transfer: {start(key:string,name:string,payload:ResourceJson,options?:{recipient?:string,timeout_ms?:number}|null):void;cancel(key:string):void};
  entity(command: ResourceJson): void;
  teleport(player: string, destination: {position: number[], heading?: number, velocity?: number[], instance?: number, restore_on_stop?: boolean} | {restore_previous: true}): void;
  state: { get(key: string, scope?: ResourceScope | null): ResourceJson; set(key: string, value?: ResourceJson, scope?: ResourceScope | null): void };
  /** Own typed operator settings. Requires resource.settings. Values are copies. */
  settings: { get(key: string): boolean | number | string; all(): Record<string, boolean | number | string> };
  storage: { get(key: string): ResourceJson; set(key: string, value?: ResourceJson): void };
  services: { submit(key: string, operation: ResourceJson, timeoutMs: number): void; cancel(key: string): void };
}
declare const resource: ResourceApi;
type ResourceVoiceServer = {kind:'channel',name:string,members:string[]} | {kind:'remove_channel',name:string} | {kind:'mute',player:string,muted:boolean} | {kind:'proximity',meters:number};
type ResourceVoiceClient = {kind:'devices'} | {kind:'configure',input_device?:string,output_device?:string,muted:boolean,deafened:boolean} | {kind:'transmit',pressed:boolean,channel?:string};
declare const sdk: {
  resource: ResourceApi;
  log(text: string): void;
  readText(path: string): string;
  submit(command: ResourceJson): void;
  animation: {version:1;submit(operation:ResourceJson):void};
  ui?: {text(key: string, text: string): void; remove(key: string): void; menu(key: string, options: ResourceJson): void; canvas(key: string, options: ResourceJson): void};
};
declare function setTimeout(callback: () => void, milliseconds?: number): number;
declare function clearTimeout(id: number): boolean;
declare function setTick(callback: () => void): number;
declare function clearTick(id: number): boolean;
declare function GetGameTimer(): number;
declare const on: ResourceApi['on'];
declare const onNet: ResourceApi['onNet'];
declare const emit: ResourceApi['emit'];
declare function emitNet(name: string, payload?: ResourceJson): void;
declare function emitNet(name: string, recipient: string | -1, payload?: ResourceJson): void;
declare const exports: ResourceApi['export'] & Record<string, Record<string, (payload?: ResourceJson) => ResourceJson>>;
