import { createFileRoute } from '@tanstack/react-router'
import { useQuery, useMutation } from '@tanstack/react-query'
import axios from 'axios'
import { Terminal } from '../../components/ui/terminal'
import { Button } from '../../components/ui/button'
import { Badge } from '../../components/ui/badge'
import { Card, CardHeader, CardTitle, CardContent } from '../../components/ui/card'
import { Loader2, XCircle } from 'lucide-react'
import { useEffect, useState } from 'react'
import { API_BASE_URL } from '../../lib/config'

export const Route = createFileRoute('/jobs/$id')({
  component: JobDetails,
})

function JobDetails() {
  const { id } = Route.useParams()
  const [logs, setLogs] = useState<string[]>([])

  const { data: job, isLoading, refetch } = useQuery({
    queryKey: ['job', id],
    queryFn: async () => {
      const res = await axios.get(`${API_BASE_URL}/jobs/${id}`)
      return res.data
    },
    refetchInterval: (query) => {
        const status = query.state.data?.status
        return (status === 'Completed' || status?.Failed) ? false : 1000
    }
  })

  const cancelMutation = useMutation({
    mutationFn: async () => {
      await axios.post(`${API_BASE_URL}/jobs/${id}/cancel`)
    },
    onSuccess: () => {
      refetch()
    }
  })

  useEffect(() => {
    const eventSource = new EventSource(`${API_BASE_URL}/jobs/${id}/logs`)
    
    eventSource.onmessage = (event) => {
      setLogs(prev => [...prev, event.data])
    }

    eventSource.onerror = () => {
      eventSource.close()
    }

    return () => {
      eventSource.close()
    }
  }, [id])

  if (isLoading) return <div className="flex justify-center p-8"><Loader2 className="animate-spin" /></div>
  if (!job) return <div>Job not found</div>

  const isRunning = job.status === 'Running' || job.status === 'Queued'

  return (
    <div className="space-y-6">
        <Card>
            <CardHeader className="flex flex-row items-center justify-between">
                <div className="space-y-1">
                    <CardTitle>Job {id.slice(0, 8)}</CardTitle>
                    <div className="text-sm text-muted-foreground">{job.flake_ref || "Local Flake"}</div>
                </div>
                <div className="flex items-center gap-4">
                    {isRunning && (
                        <Button 
                            variant="destructive" 
                            size="sm" 
                            onClick={() => cancelMutation.mutate()}
                            disabled={cancelMutation.isPending}
                        >
                            {cancelMutation.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : <XCircle className="h-4 w-4 mr-2" />}
                            Cancel
                        </Button>
                    )}
                    <Badge variant={job.status === 'Completed' ? 'success' : job.status === 'Running' ? 'default' : job.status === 'Queued' ? 'secondary' : 'destructive'}>
                        {typeof job.status === 'string' ? job.status : 'Failed'}
                    </Badge>
                </div>
            </CardHeader>
            <CardContent>
                <div className="grid grid-cols-2 gap-4 text-sm">
                    <div>
                        <span className="font-medium">Hosts:</span> {job.hosts.join(', ')}
                    </div>
                    <div>
                        <span className="font-medium">Started:</span> {new Date(job.created_at).toLocaleString()}
                    </div>
                </div>
            </CardContent>
        </Card>

        <Terminal lines={logs} className="h-[600px]" />
    </div>
  )
}