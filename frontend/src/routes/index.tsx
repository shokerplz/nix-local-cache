import { createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import axios from 'axios'
import { Link } from '@tanstack/react-router'
import { Badge } from '../components/ui/badge'
import { Card, CardHeader, CardTitle, CardContent } from '../components/ui/card'
import { API_BASE_URL } from '../lib/config'

interface Job {
  id: string
  status: string
  created_at: number
  updated_at: number
  flake_path: string
  hosts?: string[]
}

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const { isPending, error, data } = useQuery({
    queryKey: ['jobs'],
    queryFn: async () => {
      const res = await axios.get<Job[]>(`${API_BASE_URL}/jobs`);
      return res.data;
    },
    refetchInterval: 5000,
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
