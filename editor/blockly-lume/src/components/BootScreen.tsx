import { useEffect, useState } from 'react';
import { StarIcon } from './Icons';

const LINES = [
  'MOUNT /LUME/EDITOR',
  'LOAD BLOCK REGISTRY ......... OK',
  'BIND LUME SYNTAX v0.1 ...... OK',
  'START CODE GENERATOR ....... OK',
  'WORKSPACE READY',
];

export function BootScreen({ onDone }: { onDone: () => void }) {
  const [step, setStep] = useState(0);
  const [progress, setProgress] = useState(0);

  useEffect(() => {
    const lineTimer = window.setInterval(() => setStep((n) => Math.min(n + 1, LINES.length)), 330);
    const barTimer = window.setInterval(() => setProgress((n) => Math.min(n + 5, 100)), 100);
    const done = window.setTimeout(onDone, 2350);
    return () => {
      window.clearInterval(lineTimer);
      window.clearInterval(barTimer);
      window.clearTimeout(done);
    };
  }, [onDone]);

  return (
    <div className="boot" onClick={onDone} role="button" tabIndex={0} aria-label="跳过启动动画">
      <div className="boot-core">
        <div className="boot-star">
          <StarIcon width={34} height={34} />
        </div>
        <div className="boot-word">LUME</div>
        <div className="boot-sub">BLOCKLY STUDIO · FRONTEND ONLY</div>
        <div className="boot-bar">
          <span style={{ width: `${progress}%` }} />
        </div>
        <div className="boot-log">
          {LINES.slice(0, step).map((line) => (
            <div key={line}>{line}</div>
          ))}
        </div>
      </div>
      <div className="boot-skip">CLICK TO SKIP</div>
    </div>
  );
}
