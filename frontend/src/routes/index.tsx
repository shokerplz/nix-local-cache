import { createFileRoute, Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import axios from 'axios'
import { Card, CardContent } from '../components/ui/card'
import { Badge } from '../components/ui/badge'
import { Button } from '../components/ui/button'
import { Loader2, Plus, Server } from 'lucide-react'

// Types (should be shared, but defining here for now)
interface Job {
  id: string
  hosts: string[]
  status: "Queued" | "Running" | "Completed" | "Failed" | { Failed: string }
  created_at: string
  started_at?: string
  finished_at?: string
  flake_ref?: string
}

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const { data: jobs, isLoading, error } = useQuery({
    queryKey: ['jobs'],
    queryFn: async () => {
       const { data: logFiles } = await axios.get<string[]>('http://localhost:3000/logs');
       const jobIds = logFiles.map(f => f.replace('.log', ''));
       
       const jobPromises = jobIds.map(id => 
         axios.get<Job>(`http://localhost:3000/jobs/${id}`).then(r => r.data).catch(() => null)
       );
       
       const results = await Promise.all(jobPromises);
       return results.filter(j => j !== null).sort((a, b) => new Date(b!.created_at).getTime() - new Date(a!.created_at).getTime()) as Job[];
    }
  })

  if (isLoading) return <div className="flex justify-center p-8"><Loader2 className="animate-spin" /></div>
  if (error) return <div className="text-destructive text-center">Error loading jobs. Is the backend running?</div>

  return (
    <div className="space-y-6">
      <div className="flex justify-between items-center">
        <h1 className="text-3xl font-bold tracking-tight">Builds</h1>
        <Link to="/build/new">
          <Button>
            <Plus className="mr-2 h-4 w-4" /> New Build
          </Button>
        </Link>
      </div>

      <div className="grid gap-4 md:grid-cols-1">
        {jobs?.map((job) => (
          <Link key={job.id} to="/jobs/$id" params={{ id: job.id }}>
            <Card className="hover:bg-accent/50 transition-colors cursor-pointer">
              <CardContent className="p-6 flex items-center justify-between">
                <div className="space-y-1">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-sm text-muted-foreground">{job.id.slice(0, 8)}</span>
                    <StatusBadge status={job.status} />
                  </div>
                  <div className="font-medium">{job.flake_ref || "Local Flake"}</div>
                  <div className="flex gap-2 text-sm text-muted-foreground items-center">
                    <Server className="h-3 w-3" />
                    {job.hosts.join(', ')}
                  </div>
                </div>
                <div className="text-right text-sm text-muted-foreground">
                  <div>{new Date(job.created_at).toLocaleString()}</div>
                  {job.finished_at && (
                    <div>took {Math.round((new Date(job.finished_at).getTime() - new Date(job.started_at || job.created_at).getTime()) / 1000)}s</div>
                  )}
                </div>
              </CardContent>
            </Card>
          </Link>
        ))}
        {jobs?.length === 0 && (
          <div className="text-center text-muted-foreground py-12 m-auto">No builds found.</div>
        )}
      </div>
    </div>
  )
}

function StatusBadge({ status }: { status: Job["status"] }) {
  if (status === "Completed") return <Badge variant="success">Completed</Badge>
  if (status === "Running") return <Badge variant="default" className="animate-pulse">Running</Badge>
  if (status === "Queued") return <Badge variant="secondary">Queued</Badge>
  return <Badge variant="destructive">Failed</Badge>
}
