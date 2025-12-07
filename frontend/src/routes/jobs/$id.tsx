import { createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import axios from 'axios'
import { Terminal } from '../../components/ui/terminal'
import { Badge } from '../../components/ui/badge'
import { Card, CardHeader, CardTitle, CardContent } from '../../components/ui/card'
import { Loader2 } from 'lucide-react'
import { useEffect, useState } from 'react'

export const Route = createFileRoute('/jobs/$id')({
  component: JobDetails,
})

function JobDetails() {
  const { id } = Route.useParams()
  const [logs, setLogs] = useState<string[]>([])

  const { data: job, isLoading } = useQuery({
    queryKey: ['job', id],
    queryFn: async () => {
      const res = await axios.get(`http://localhost:3000/jobs/${id}`)
      return res.data
    },
    refetchInterval: (query) => {
        const status = query.state.data?.status
        return (status === 'Completed' || status?.Failed) ? false : 1000
    }
  })

  useEffect(() => {
    const eventSource = new EventSource(`http://localhost:3000/jobs/${id}/logs`)
    
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

  return (
    <div className="space-y-6">
        <Card>
            <CardHeader className="flex flex-row items-center justify-between">
                <div className="space-y-1">
                    <CardTitle>Job {id.slice(0, 8)}</CardTitle>
                    <div className="text-sm text-muted-foreground">{job.flake_ref || "Local Flake"}</div>
                </div>
                <Badge variant={job.status === 'Completed' ? 'success' : job.status === 'Running' ? 'default' : 'destructive'}>
                    {typeof job.status === 'string' ? job.status : 'Failed'}
                </Badge>
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