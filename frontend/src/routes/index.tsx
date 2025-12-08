import { createFileRoute } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import axios from 'axios'
import { Link } from '@tanstack/react-router'
import { Badge } from '../components/ui/badge'
import { Card, CardContent } from '../components/ui/card'
import { Button } from '../components/ui/button'
import { Loader2, Plus, Server, ChevronLeft, ChevronRight } from 'lucide-react'
import { API_BASE_URL } from '../lib/config'
import { useState } from 'react'

interface Job {
  id: string
  status: string
  created_at: number
  started_at?: number
  finished_at?: number
  updated_at: number
  flake_path: string
  flake_ref?: string
  hosts: string[]
}

interface PaginatedJobs {
  jobs: Job[]
  total: number
  page: number
  page_size: number
  total_pages: number
}

export const Route = createFileRoute('/')({
  component: Dashboard,
})

function Dashboard() {
  const [page, setPage] = useState(1)
  const [pageSize, setPageSize] = useState(10)

  const { isPending, error, data: paginatedData } = useQuery({
    queryKey: ['jobs', page, pageSize],
    queryFn: async () => {
      const res = await axios.get<PaginatedJobs>(`${API_BASE_URL}/jobs?page=${page}&page_size=${pageSize}`);
      return res.data;
    },
    refetchInterval: 5000,
  })

  if (isPending) return <div className="flex justify-center p-8"><Loader2 className="animate-spin" /></div>
  if (error) return <div className="text-destructive text-center">Error loading jobs. Is the backend running?</div>

  const jobs = paginatedData?.jobs || []
  const totalPages = paginatedData?.total_pages || 1

  return (
    <div className="space-y-6">
      <div className="flex flex-col sm:flex-row justify-between items-start sm:items-center gap-4">
        <h1 className="text-2xl sm:text-3xl font-bold tracking-tight">Builds</h1>
        <Link to="/build/new" className="w-full sm:w-auto block">
          <Button className="w-full sm:w-auto">
            <Plus className="mr-2 h-4 w-4" /> New Build
          </Button>
        </Link>
      </div>

      <div className="flex flex-col gap-4">
        {jobs.map((job) => (
          <Link key={job.id} to="/jobs/$id" params={{ id: job.id }} className="block">
            <Card className="hover:bg-accent/50 transition-colors cursor-pointer w-full">
              <CardContent className="p-4 sm:p-6 flex flex-col sm:flex-row sm:items-center justify-between gap-4">
                <div className="flex-1 space-y-1 w-full min-w-0">
                  <div className="flex items-center gap-2 flex-wrap">
                    <span className="font-mono text-sm text-muted-foreground">{job.id.slice(0, 8)}</span>
                    <StatusBadge status={job.status} />
                  </div>
                  <div className="font-medium truncate">{job.flake_ref || "Local Flake"}</div>
                  <div className="flex gap-2 text-sm text-muted-foreground items-center flex-wrap">
                    <Server className="h-3 w-3 flex-shrink-0" />
                    <span className="break-all">{job.hosts.join(', ')}</span>
                  </div>
                </div>
                <div className="text-left sm:text-right text-sm text-muted-foreground flex-shrink-0 w-full sm:w-auto">
                  <div>{new Date(job.created_at).toLocaleString()}</div>
                  {job.finished_at && (
                    <div>took {Math.round((new Date(job.finished_at).getTime() - new Date(job.started_at || job.created_at).getTime()) / 1000)}s</div>
                  )}
                </div>
              </CardContent>
            </Card>
          </Link>
        ))}
        {jobs.length === 0 && (
          <div className="text-center text-muted-foreground py-12 m-auto">No builds found.</div>
        )}
      </div>

      <div className="flex items-center justify-center gap-4 py-4 flex-wrap">
        <div className="flex items-center gap-2">
            <span className="text-sm text-muted-foreground">Rows per page</span>
            <select
                className="h-8 rounded-md border border-input bg-background px-2 py-1 text-sm ring-offset-background focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2"
                value={pageSize}
                onChange={(e) => {
                    setPageSize(Number(e.target.value))
                    setPage(1)
                }}
            >
                <option value={10}>10</option>
                <option value={25}>25</option>
                <option value={50}>50</option>
            </select>
        </div>
        <div className="flex items-center gap-2">
            <Button
                variant="outline"
                size="sm"
                onClick={() => setPage((p) => Math.max(1, p - 1))}
                disabled={page === 1 || isPending}
            >
                <ChevronLeft className="h-4 w-4 mr-1" /> Previous
            </Button>
            <span className="text-sm text-muted-foreground min-w-[5rem] text-center">
                Page {page} of {totalPages}
            </span>
            <Button
                variant="outline"
                size="sm"
                onClick={() => setPage((p) => Math.min(totalPages, p + 1))}
                disabled={page >= totalPages || isPending}
            >
                Next <ChevronRight className="h-4 w-4 ml-1" />
            </Button>
        </div>
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
