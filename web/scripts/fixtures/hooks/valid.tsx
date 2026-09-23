import { useEffect, useState } from 'react'

export function Valid({ value }: { value: number }) {
  const [state] = useState(0)
  useEffect(() => { console.log(value) }, [value])
  return <span>{state}</span>
}
