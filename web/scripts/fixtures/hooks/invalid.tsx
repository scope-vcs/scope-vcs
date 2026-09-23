import { useEffect, useState } from 'react'

export function Invalid({ value }: { value: number }) {
  if (value > 0) useState(0)
  useEffect(() => { console.log(value) }, [])
  return null
}
