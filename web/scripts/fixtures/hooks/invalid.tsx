import { useEffect, useState } from 'react'

export function Invalid({ value }: { value: number }) {
  if (value > 0) useState(0)
  useEffect(() => { document.title = String(value) }, [])
  return null
}
