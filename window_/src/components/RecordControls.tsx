import { useEffect, useState } from "react";
import { api } from "../api";
import { useRecorder } from "../store";

export default function RecordControls() {
  const {
    selectedPid,
    micMix,
    status,
    defaultDir,
    setMicMix,
    setStatus,
    setError,
  } = useRecorder();
  const [elapsed, setElapsed] = useState(0);

  const isRecording = status.kind === "recording";

  useEffect(() => {
    if (!isRecording) {
      setElapsed(0);
      return;
    }
    const start = new Date(status.started_at_iso).getTime();
    const tick = () => setElapsed(Math.max(0, (Date.now() - start) / 1000));
    tick();
    const id = window.setInterval(tick, 250);
    return () => window.clearInterval(id);
  }, [isRecording, status]);

  const startStop = async () => {
    if (isRecording) {
      try {
        const meta = await api.stopRecording();
        // eslint-disable-next-line no-console
        console.info(`Saved ${meta.output_path} (${meta.duration_seconds.toFixed(1)}s)`);
      } catch (e) {
        setError(`Arrêt impossible : ${e}`);
      } finally {
        setStatus({ kind: "idle" });
      }
      return;
    }
    if (selectedPid == null) {
      setError("Sélectionne d’abord l’application de l’appel.");
      return;
    }
    try {
      const result = await api.startRecording({
        pid: selectedPid,
        mic_mix: micMix,
      });
      setStatus(result);
    } catch (e) {
      setError(`Démarrage impossible : ${e}`);
    }
  };

  return (
    <section className="controls">
      <div className="control-row">
        <label className="mic-toggle">
          <input
            type="checkbox"
            checked={micMix}
            onChange={(e) => setMicMix(e.target.checked)}
            disabled={isRecording}
          />
          Inclure mon microphone
        </label>
        <span className="size-hint">MP3 mono 64 kbit/s · ~29 Mo/heure</span>
      </div>
      <p className="output-path">
        {defaultDir ? `Dossier : ${truncate(defaultDir)}` : "Préparation du dossier local…"}
      </p>
      <button
        className={`record-button ${isRecording ? "stop" : "start"}`}
        onClick={startStop}
        aria-live="polite"
      >
        {isRecording ? `Arrêter (${formatTime(elapsed)})` : "Enregistrer l’appel"}
      </button>
    </section>
  );
}

function truncate(p: string, max = 56): string {
  if (p.length <= max) return p;
  return "…" + p.slice(p.length - max + 1);
}

function formatTime(secs: number): string {
  const m = Math.floor(secs / 60);
  const s = Math.floor(secs % 60);
  return `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
}
