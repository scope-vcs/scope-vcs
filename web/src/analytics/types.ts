export type Property = unknown
export type Properties = Record<string, Property>

export type CaptureResult = {
  event: string
  properties: Properties
  uuid: string
  timestamp?: string
  $set?: Properties
  $set_once?: Properties
  $unset?: string[]
}
