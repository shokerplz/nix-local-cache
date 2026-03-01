import { createFileRoute, useNavigate } from '@tanstack/react-router'
import { useMutation } from '@tanstack/react-query'
import axios from 'axios'
import { Card, CardContent, CardHeader, CardTitle } from '../../components/ui/card'
import { Button } from '../../components/ui/button'
import { Input } from '../../components/ui/input'
import { Badge } from '../../components/ui/badge'
import { Loader2, RefreshCw, X } from 'lucide-react'
import React, { useState } from 'react'
import { API_BASE_URL } from '../../lib/config'

interface BuildPayload {
  flake_url?: string
  flake_branch?: string
  hosts?: string[]
  timeout_seconds?: number
}

export const Route = createFileRoute('/build/new')({
  component: NewBuild,
})

function NewBuild() {
  const navigate = useNavigate()
  const [flakeUrl, setFlakeUrl] = useState('')
  const [branch, setBranch] = useState('')
  const [hosts, setHosts] = useState('')
  const [timeoutSeconds, setTimeoutSeconds] = useState('')
  
  const [availableHosts, setAvailableHosts] = useState<string[]>([])
  const [selectedHosts, setSelectedHosts] = useState<Set<string>>(new Set())
  const [isFetchingHosts, setIsFetchingHosts] = useState(false)
  const [fetchError, setFetchError] = useState('')

  const mutation = useMutation({
    mutationFn: async () => {
      const payload: BuildPayload = {}
      if (flakeUrl) payload.flake_url = flakeUrl
      if (branch) payload.flake_branch = branch

      const timeoutValue = timeoutSeconds.trim()
      if (timeoutValue) {
        const parsedTimeout = Number.parseInt(timeoutValue, 10)
        if (!Number.isInteger(parsedTimeout) || parsedTimeout <= 0) {
          throw new Error('Timeout must be a positive number of seconds.')
        }
        payload.timeout_seconds = parsedTimeout
      }
      
      if (availableHosts.length > 0) {
        // If using selection mode, use selected hosts
        // If selection is empty, we might want to warn or send nothing?
        // User said "by default build all".
        // If we fetched hosts, and user selected nothing, maybe they mean nothing?
        // But if they just fetched and hit submit, they might expect all.
        // Let's enforce: if availableHosts > 0, use selectedHosts.
        // To support "Build All", we should select all by default on fetch.
        if (selectedHosts.size === 0) {
             throw new Error("Please select at least one host to build.")
        }
        payload.hosts = Array.from(selectedHosts)
      } else {
        // Manual input mode
        if (hosts) payload.hosts = hosts.split(',').map(h => h.trim()).filter(h => h.length > 0)
      }

      const res = await axios.post(`${API_BASE_URL}/build`, payload)
      return res.data
    },
    onSuccess: (data) => {
      navigate({ to: "/jobs/$id", params: { id: data.job_id } })
    }
  })

  const fetchHosts = async () => {
    setIsFetchingHosts(true)
    setFetchError('')
    try {
      const params = new URLSearchParams()
      if (flakeUrl) params.append('flake_url', flakeUrl)
      if (branch) params.append('branch', branch)
      
      const res = await axios.get<string[]>(`${API_BASE_URL}/flake/hosts?${params.toString()}`)
      const hosts = res.data
      setAvailableHosts(hosts)
      // Select all by default
      setSelectedHosts(new Set(hosts))
    } catch (e: any) {
      console.error(e)
      setFetchError(e.response?.data || e.message || 'Failed to fetch hosts')
    } finally {
      setIsFetchingHosts(false)
    }
  }

  const toggleHost = (host: string) => {
    const newSelected = new Set(selectedHosts)
    if (newSelected.has(host)) {
      newSelected.delete(host)
    } else {
      newSelected.add(host)
    }
    setSelectedHosts(newSelected)
  }
  
  const clearHosts = () => {
    setAvailableHosts([])
    setSelectedHosts(new Set())
    setFetchError('')
  }
  
  const selectAll = () => {
    setSelectedHosts(new Set(availableHosts))
  }
  
  const selectNone = () => {
    setSelectedHosts(new Set())
  }

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault()
    mutation.mutate()
  }

  return (
    <div className="max-w-2xl mx-auto">
      <Card>
        <CardHeader className="p-4 sm:p-6">
          <CardTitle className="text-xl">Trigger New Build</CardTitle>
        </CardHeader>
        <CardContent className="px-4 sm:px-6">
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
              <label className="text-sm font-medium">Timeout in Seconds (Optional)</label>
              <Input
                type="number"
                min={1}
                step={1}
                placeholder="43200"
                value={timeoutSeconds}
                onChange={e => setTimeoutSeconds(e.target.value)}
              />
              <p className="text-xs text-muted-foreground">Defaults to 43200 seconds (12 hours) when left empty.</p>
            </div>

            <div className="space-y-2">
              <div className="flex justify-between items-center">
                <label className="text-sm font-medium">Hosts</label>
                {availableHosts.length === 0 ? (
                    <Button 
                        type="button" 
                        variant="ghost" 
                        size="sm" 
                        onClick={fetchHosts} 
                        disabled={isFetchingHosts}
                        className="h-8 text-xs"
                    >
                        {isFetchingHosts ? <Loader2 className="mr-2 h-3 w-3 animate-spin" /> : <RefreshCw className="mr-2 h-3 w-3" />}
                        Fetch Hosts
                    </Button>
                ) : (
                    <div className="flex gap-2">
                         <Button type="button" variant="ghost" size="sm" onClick={selectAll} className="h-6 text-xs">All</Button>
                         <Button type="button" variant="ghost" size="sm" onClick={selectNone} className="h-6 text-xs">None</Button>
                         <Button type="button" variant="ghost" size="sm" onClick={clearHosts} className="h-6 text-xs"><X className="h-3 w-3" /></Button>
                    </div>
                )}
              </div>
              
              {fetchError && (
                  <p className="text-xs text-destructive">{fetchError}</p>
              )}

              {availableHosts.length > 0 ? (
                <div className="flex flex-wrap gap-2 p-3 border rounded-md min-h-[50px]">
                  {availableHosts.map(host => (
                    <Badge
                      key={host}
                      variant={selectedHosts.has(host) ? "default" : "outline"}
                      className="cursor-pointer hover:opacity-80 transition-all select-none"
                      onClick={() => toggleHost(host)}
                    >
                      {host}
                    </Badge>
                  ))}
                </div>
              ) : (
                <>
                  <Input
                    placeholder="media-server, rpi5"
                    value={hosts}
                    onChange={e => setHosts(e.target.value)}
                  />
                  <p className="text-xs text-muted-foreground">Leave empty to build all hosts in the flake.</p>
                </>
              )}
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
