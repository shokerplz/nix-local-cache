import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useMutation } from '@tanstack/react-query'
import axios from 'axios'
import { Card, CardContent, CardHeader, CardTitle } from '../../components/ui/card'
import { Button } from '../../components/ui/button'
import { Input } from '../../components/ui/input'
import { Loader2 } from 'lucide-react'
import React, { useState } from 'react'

interface BuildPayload {
  flake_url?: string
  flake_branch?: string
  hosts?: string[]
}

export const Route = createFileRoute('/build/new')({
  component: NewBuild,
})

function NewBuild() {
  const navigate = useNavigate()
  const [flakeUrl, setFlakeUrl] = useState('')
  const [branch, setBranch] = useState('')
  const [hosts, setHosts] = useState('')

  const mutation = useMutation({
    mutationFn: async () => {
      const payload: BuildPayload = {}
      if (flakeUrl) payload.flake_url = flakeUrl
      if (branch) payload.flake_branch = branch
      if (hosts) payload.hosts = hosts.split(',').map(h => h.trim()).filter(h => h.length > 0)
      
      const res = await axios.post('http://localhost:3000/build', payload)
      return res.data
    },
    onSuccess: (data) => {
      navigate({ to: "/jobs/$id", params: { id: data.job_id } })
    }
  })

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    mutation.mutate()
  }

  return (
    <div className="max-w-2xl mx-auto">
      <Card>
        <CardHeader>
          <CardTitle>Trigger New Build</CardTitle>
        </CardHeader>
        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div className="space-y-2">
              <label className="text-sm font-medium">Flake URL (Optional)</label>
              <Input 
                placeholder="git+https://github.com/owner/repo.git" 
                value={flakeUrl}
                onChange={e => setFlakeUrl(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">Leave empty to use server-configured local flake.</p>
            </div>
            
            <div className="space-y-2">
              <label className="text-sm font-medium">Branch / Ref (Optional)</label>
              <Input 
                placeholder="main" 
                value={branch}
                onChange={e => setBranch(e.target.value)}
              />
            </div>

            <div className="space-y-2">
              <label className="text-sm font-medium">Hosts (Optional, comma separated)</label>
              <Input 
                placeholder="media-server, rpi5" 
                value={hosts}
                onChange={e => setHosts(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">Leave empty to build all hosts in the flake.</p>
            </div>

            <div className="pt-4">
              <Button type="submit" disabled={mutation.isPending} className="w-full">
                {mutation.isPending && <Loader2 className="mr-2 h-4 w-4 animate-spin" />}
                Start Build
              </Button>
            </div>
            
            {mutation.isError && (
              <div className="text-sm text-destructive mt-2">
                Failed to start build: {mutation.error.message}
              </div>
            )}
          </form>
        </CardContent>
      </Card>
    </div>
  )
}