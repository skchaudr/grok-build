import Navbar from "@/components/landing/Navbar";
import Hero from "@/components/landing/Hero";
import Features from "@/components/landing/Features";
import Comparison from "@/components/landing/Comparison";
import Architecture from "@/components/landing/Architecture";
import TerminalDemo from "@/components/landing/TerminalDemo";
import Migration from "@/components/landing/Migration";
import Download from "@/components/landing/Download";
import Footer from "@/components/landing/Footer";

export default function Home() {
  return (
    <>
      <Navbar />
      <main>
        <Hero />
        <Features />
        <Comparison />
        <Architecture />
        <TerminalDemo />
        <Migration />
        <Download />
      </main>
      <Footer />
    </>
  );
}
